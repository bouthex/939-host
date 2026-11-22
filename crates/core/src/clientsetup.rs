//! «Preparar para jugar»: deja listo el Minecraft de esta PC para entrar a un servidor, sin que el
//! usuario baje ni importe nada.
//!
//! Qué hace, en orden:
//! 1. Encuentra el Launcher oficial (`%APPDATA%\.minecraft` con `launcher_profiles.json`; también
//!    la versión de Microsoft Store, `launcher_profiles_microsoft_store.json`).
//! 2. Instala la versión de Minecraft si falta (JSON y cliente oficiales de Mojang, con SHA-1). El
//!    resto (librerías, sonidos) lo completa el Launcher al abrir el perfil, como siempre.
//! 3. Instala el cargador si falta: Fabric con su perfil oficial (meta.fabricmc.net); Forge y
//!    NeoForge con su instalador oficial en modo cliente (huella SHA-1 del Maven oficial), con el
//!    Java que ya maneja 939 Host.
//! 4. Usa una carpeta de juego propia por servidor (`.minecraft\939 Host\<servidor>`, o la que ya
//!    tenga su perfil en el Launcher): los mods, mundos y opciones de otros perfiles nunca se
//!    mezclan ni se tocan. Si esa carpeta ya tenía `mods`, se renombra a «mods anteriores» (o
//!    «mods anteriores 1», «2», …) y se crea una nueva (se puede deshacer).
//!    La primera vez, la carpeta nueva recibe una COPIA de las preferencias del último perfil que
//!    usaste (idioma, teclas, accesibilidad, gráficos, sonido: `options.txt` y compañía) y de tu
//!    lista de servidores; los originales no se modifican. Los mundos no se copian ni se mueven:
//!    siguen en su perfil (abrir un mundo con otra versión puede dañarlo).
//! 5. Baja los mods que necesita el cliente (los de Modrinth con su hash; los que el servidor tiene
//!    a mano se copian desde la PC que tiene el servidor). Los que solo van en el servidor no.
//! 6. Crea o actualiza el perfil «939 Host · <servidor>» en el Launcher (con copia previa del
//!    archivo de perfiles) y agrega el servidor a la lista de Multijugador de ese perfil.
//! 7. Comprueba todo (versión, cargador, cada mod por hash, perfil) antes de decir que está listo.
//!
//! Si algo falla a mitad de camino, los mods nuevos se descartan y la carpeta anterior vuelve a su
//! lugar: no queda una instalación a medias.

use crate::error::{AppError, AppResult};
use crate::model::Loader;
use crate::{java, meta, net, paths};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Lo que necesita el cliente para un servidor. Lo arma la PC que tiene el servidor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub server_id: String,
    pub server_name: String,
    pub mc_version: String,
    pub loader: Loader,
    pub loader_version: Option<String>,
    pub mods: Vec<PlanMod>,
    /// Mods del servidor que no van en el cliente (solo para informar).
    pub server_only: Vec<String>,
    /// Resource packs opcionales activos: se instalan y activan en el perfil del jugador.
    #[serde(default)]
    pub packs: Vec<PlanMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanMod {
    pub filename: String,
    pub title: String,
    pub sha1: String,
    pub size: u64,
    /// Descarga oficial (Modrinth). `None` = mod agregado a mano: se copia desde el servidor.
    pub url: Option<String>,
}

impl Default for Loader { fn default() -> Self { Loader::Vanilla } }

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Para mostrar: «Minecraft 1.21.4 con Fabric 0.16.9».
    pub label: String,
    pub profile_name: String,
    pub version_id: String,
    pub game_dir: String,
    pub mods_installed: usize,
    pub previous_mods: Option<String>,
    pub server_only: Vec<String>,
    pub installed_version: bool,
    pub installed_loader: bool,
    /// Perfil del que se copiaron las preferencias (solo la primera vez).
    pub seeded_from: Option<String>,
    /// Archivos de preferencias copiados (`options.txt`, `servers.dat`, …).
    pub seeded: Vec<String>,
    /// Servidores en la lista de Multijugador del perfil.
    pub servers_listed: usize,
    /// Resource packs opcionales instalados en el perfil.
    pub packs: Vec<String>,
    /// Perfil ya existente sin tus preferencias: nombre del perfil del que se pueden tomar.
    pub prefs_available: Option<String>,
}

/// Carpeta del Minecraft de esta PC: la elegida a mano (si es válida) o la detectada (ver
/// `installations`). `H939_MINECRAFT_DIR` solo para pruebas (nunca la real).
pub fn minecraft_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("H939_MINECRAFT_DIR") { return Some(PathBuf::from(d)); }
    let custom = crate::settings::load().minecraft_dir;
    if !custom.trim().is_empty() && looks_like_minecraft(Path::new(&custom)) { return Some(PathBuf::from(custom)); }
    installations().into_iter().next().map(|i| i.path).or_else(default_candidates_first)
}

fn default_candidates_first() -> Option<PathBuf> { candidates().into_iter().next().map(|(p, _)| p) }

/// Lugares donde el Launcher oficial guarda sus datos.
fn candidates() -> Vec<(PathBuf, &'static str)> {
    let mut v = vec![];
    #[cfg(windows)]
    {
        if let Some(a) = std::env::var_os("APPDATA") { v.push((PathBuf::from(a).join(".minecraft"), "Launcher de Minecraft")); }
        // Launcher de Microsoft Store: Windows puede guardar sus datos en la carpeta virtual del paquete.
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push((PathBuf::from(l).join(r"Packages\Microsoft.4297127D64EC6_8wekyb3d8bbwe\LocalCache\Roaming\.minecraft"), "Launcher de Microsoft Store"));
        }
    }
    #[cfg(not(windows))]
    { if let Some(h) = std::env::var_os("HOME") { v.push((PathBuf::from(h).join(".minecraft"), "Launcher de Minecraft")); } }
    v
}

/// ¿Es una carpeta de datos del Launcher? (perfiles o versiones instaladas).
pub fn looks_like_minecraft(p: &Path) -> bool { !launcher_files(p).is_empty() || p.join("versions").is_dir() }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Install { pub path: PathBuf, pub source: String, pub has_launcher: bool, pub profiles: usize }

/// Instalaciones de Minecraft encontradas, la más usada recientemente primero (la del archivo de
/// perfiles modificado más tarde). No crea nada.
pub fn installations() -> Vec<Install> {
    let mut v: Vec<(std::time::SystemTime, Install)> = candidates().into_iter().filter(|(p, _)| looks_like_minecraft(p)).map(|(p, src)| {
        let files = launcher_files(&p);
        let newest = files.iter().filter_map(|f| f.metadata().and_then(|m| m.modified()).ok()).max().unwrap_or(std::time::UNIX_EPOCH);
        let profiles = files.iter().filter_map(|f| std::fs::read_to_string(f).ok()).filter_map(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).map(|v| v["profiles"].as_object().map(|o| o.len()).unwrap_or(0)).max().unwrap_or(0);
        (newest, Install { has_launcher: !files.is_empty(), path: p, source: src.into(), profiles })
    }).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.dedup_by(|a, b| a.1.path == b.1.path);
    v.into_iter().map(|(_, i)| i).collect()
}

const PROFILE_FILES: [&str; 2] = ["launcher_profiles.json", "launcher_profiles_microsoft_store.json"];

/// Archivos de perfiles del Launcher que existen (al menos uno, o el Launcher no está).
pub fn launcher_files(mc: &Path) -> Vec<PathBuf> { PROFILE_FILES.iter().map(|f| mc.join(f)).filter(|p| p.is_file()).collect() }

/// Nombre de carpeta seguro para Windows a partir del nombre del servidor.
pub fn folder_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '(' | ')') { c } else { '-' }).collect();
    let s = s.trim().trim_matches('.').trim().to_string();
    let s: String = s.chars().take(60).collect();
    if s.is_empty() { "servidor".into() } else { s }
}

/// Instancia del servidor dentro de `.minecraft` (lugar por defecto, sin mirar si ya existe).
pub fn game_dir(mc: &Path, plan: &Plan) -> PathBuf { mc.join("939 Host").join(folder_name(&plan.server_name)) }

const MARKER: &str = ".939host-perfil.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Marker { server_id: String, server_name: String, #[serde(default)] seeded_from: Option<String> }

fn read_marker(g: &Path) -> Option<Marker> { paths::read_json(&g.join(MARKER)).ok() }

/// Perfiles del Launcher (todos los archivos de perfiles): clave → (nombre, carpeta de juego, último uso).
fn launcher_profiles(mc: &Path) -> Vec<(String, String, PathBuf, String)> {
    let mut out = vec![];
    for f in launcher_files(mc) {
        let Ok(t) = std::fs::read_to_string(&f) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
        for (k, p) in v["profiles"].as_object().into_iter().flatten() {
            let gd = p["gameDir"].as_str().filter(|g| !g.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| mc.to_path_buf());
            let name = p["name"].as_str().filter(|n| !n.is_empty()).unwrap_or_else(|| p["lastVersionId"].as_str().unwrap_or(k)).to_string();
            out.push((k.clone(), name, gd, p["lastUsed"].as_str().unwrap_or("").to_string()));
        }
    }
    out
}

/// Carpeta de juego de este servidor: la que ya usa su perfil en el Launcher (aunque la persona la
/// haya cambiado), o una nueva que no sea de ningún otro perfil (dos servidores con el mismo nombre
/// no comparten carpeta).
pub fn resolve_game_dir(mc: &Path, plan: &Plan) -> PathBuf {
    let key = format!("939host-{}", plan.server_id);
    let profiles = launcher_profiles(mc);
    if let Some((_, _, g, _)) = profiles.iter().find(|(k, _, g, _)| *k == key && g.is_dir() && g != mc) { return g.clone(); }
    let base = game_dir(mc, plan);
    for i in 1..1000 {
        let c = if i == 1 { base.clone() } else { base.with_file_name(format!("{} ({i})", folder_name(&plan.server_name))) };
        if !c.exists() { return c; }
        match read_marker(&c) {
            Some(m) if m.server_id == plan.server_id => return c,
            Some(_) => continue,
            // Sin marca (creada por una versión anterior): es de este servidor salvo que otro perfil la use.
            None => if !profiles.iter().any(|(k, _, g, _)| *k != key && g == &c) { return c; },
        }
    }
    base
}

/// Perfil que la persona usó por última vez (para tomar sus preferencias), sin contar `except`.
pub fn last_used_profile(mc: &Path, except: &Path) -> Option<(String, PathBuf)> {
    launcher_profiles(mc).into_iter().filter(|(_, _, g, _)| g != except && g.is_dir()).filter(|(_, _, g, _)| PREF_FILES.iter().any(|f| g.join(f).is_file()) || g.join("servers.dat").is_file())
        .max_by(|a, b| a.3.cmp(&b.3)).map(|(_, n, g, _)| (n, g))
}

/// Preferencias personales que se copian a un perfil nuevo: opciones del juego (idioma, teclas,
/// accesibilidad, gráficos, sonido, chat), las de OptiFine y shaders, y las barras guardadas.
/// Marca que el perfil ya recibió las preferencias de `from` (para no ofrecerlo otra vez).
pub fn mark_seeded(gdir: &Path, from: &str) -> AppResult<()> {
    let mut m = read_marker(gdir).unwrap_or_default();
    m.seeded_from = Some(from.to_string());
    paths::write_json(&gdir.join(MARKER), &m)
}

pub const PREF_FILES: [&str; 4] = ["options.txt", "optionsof.txt", "optionsshaders.txt", "hotbar.nbt"];

/// Copia las preferencias del perfil de referencia a `gdir` sin pisar nada que ya exista ahí.
/// Devuelve lo copiado. La lista de servidores se copia completa (con íconos y elección de packs).
pub fn seed_preferences(from: &Path, gdir: &Path) -> AppResult<Vec<String>> {
    let mut done = vec![];
    for f in PREF_FILES.iter().chain(std::iter::once(&"servers.dat")) {
        let (src, dst) = (from.join(f), gdir.join(f));
        if !src.is_file() || dst.exists() { continue; }
        let tmp = gdir.join(format!(".{f}.939host-copia"));
        std::fs::copy(&src, &tmp).map_err(|e| AppError::io(&format!("No se pudo copiar {f} del perfil anterior"), e))?;
        if std::fs::read(&tmp).ok() != std::fs::read(&src).ok() { let _ = std::fs::remove_file(&tmp); return Err(AppError::invalid(format!("La copia de {f} no quedó igual; no se usó."))); }
        paths::rename_retry(&tmp, &dst).map_err(|e| AppError::io(&format!("No se pudo guardar {f}"), e))?;
        done.push(f.to_string());
    }
    Ok(done)
}

/// Usa las preferencias del perfil `from` en un perfil que ya existía (por ejemplo, uno creado por
/// una versión anterior de 939 Host que arrancó con las opciones por defecto). Lo que había no se
/// borra: queda como `<archivo>.antes-939host` (o numerado). La lista de servidores no se reemplaza:
/// se le agregan los servidores del otro perfil que falten. Devuelve lo cambiado.
pub fn import_preferences(from: &Path, gdir: &Path) -> AppResult<Vec<String>> {
    let mut done = vec![];
    for f in PREF_FILES {
        let (src, dst) = (from.join(f), gdir.join(f));
        if !src.is_file() { continue; }
        if std::fs::read(&src).ok() == std::fs::read(&dst).ok() { continue; }
        if dst.exists() {
            let mut bk = gdir.join(format!("{f}.antes-939host"));
            let mut n = 1;
            while bk.exists() { n += 1; bk = gdir.join(format!("{f}.antes-939host-{n}")); }
            std::fs::copy(&dst, &bk).map_err(|e| AppError::io("No se pudo guardar una copia de tus opciones actuales; no se cambió nada", e))?;
            if std::fs::read(&bk).ok() != std::fs::read(&dst).ok() { return Err(AppError::invalid("La copia de tus opciones actuales no quedó igual; no se cambió nada.")); }
        }
        let tmp = gdir.join(format!(".{f}.939host-copia"));
        std::fs::copy(&src, &tmp).map_err(|e| AppError::io("No se pudo copiar las preferencias", e))?;
        paths::rename_retry(&tmp, &dst).map_err(|e| AppError::io("No se pudo guardar las preferencias", e))?;
        done.push(f.to_string());
    }
    // Servidores del otro perfil que falten (con su ícono y su elección de packs).
    if let Ok(v) = crate::nbt::read_nbt_file(&from.join("servers.dat")) {
        let mut added = 0;
        for e in v["servers"].as_array().into_iter().flatten().rev() {
            let (Some(name), Some(ip)) = (e["name"].as_str(), e["ip"].as_str()) else { continue };
            let have = crate::nbt::read_nbt_file(&gdir.join("servers.dat")).ok().map(|d| d["servers"].as_array().into_iter().flatten().any(|x| x["ip"].as_str() == Some(ip))).unwrap_or(false);
            if have { continue; }
            add_server_entry_full(gdir, name, ip, e["icon"].as_str(), e["acceptTextures"].as_i64())?;
            added += 1;
        }
        if added > 0 { done.push(format!("servers.dat (+{added})")); }
    }
    Ok(done)
}

fn add_server_entry_full(gdir: &Path, name: &str, ip: &str, icon: Option<&str>, accept: Option<i64>) -> AppResult<()> {
    let s = |b: &mut Vec<u8>, t: &str| { b.extend((t.len() as u16).to_be_bytes()); b.extend(t.as_bytes()); };
    let mut entry = vec![];
    entry.push(8); s(&mut entry, "name"); s(&mut entry, name);
    entry.push(8); s(&mut entry, "ip"); s(&mut entry, ip);
    if let Some(i) = icon { entry.push(8); s(&mut entry, "icon"); s(&mut entry, i); }
    if let Some(a) = accept { entry.push(1); s(&mut entry, "acceptTextures"); entry.push(a as u8); }
    entry.push(0);
    write_server_entry(gdir, ip, &entry)
}

/// Activa un resource pack en `options.txt` (lo agrega al final de `resourcePacks`, sin quitar nada).
pub fn enable_resource_pack(gdir: &Path, file_name: &str) -> AppResult<()> {
    let p = gdir.join("options.txt");
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let entry = format!("file/{file_name}");
    let mut found = false;
    let mut lines: Vec<String> = text.lines().map(|l| {
        if let Some(v) = l.strip_prefix("resourcePacks:") {
            found = true;
            let mut list: Vec<String> = serde_json::from_str(v).unwrap_or_else(|_| vec!["vanilla".into()]);
            if !list.contains(&entry) { list.push(entry.clone()); }
            return format!("resourcePacks:{}", serde_json::to_string(&list).unwrap());
        }
        l.to_string()
    }).collect();
    if !found { lines.push(format!("resourcePacks:{}", serde_json::to_string(&vec!["vanilla".to_string(), entry]).unwrap())); }
    let out = lines.join("\n") + "\n";
    if out != text { paths::write_atomic(&p, out.as_bytes())?; }
    Ok(())
}

/// Deshace el cambio de mods: la carpeta `previous` vuelve a ser `mods` y la que puso 939 Host
/// queda aparte (con otro nombre). No se borra nada. Devuelve el nombre de la carpeta apartada.
pub fn undo_mods(gdir: &Path, previous: &str) -> AppResult<String> {
    if previous.contains(['/', '\\']) || !previous.starts_with("mods anteriores") { return Err(AppError::invalid("Carpeta inválida.")); }
    let prev = gdir.join(previous);
    if !prev.is_dir() { return Err(AppError::not_found("la carpeta de mods anterior")); }
    let mods = gdir.join("mods");
    let aside = previous_name(gdir);
    if mods.exists() { paths::rename_retry(&mods, &aside).map_err(|e| AppError::io("No se pudo apartar la carpeta de mods actual", e))?; }
    if let Err(e) = paths::rename_retry(&prev, &mods) {
        if aside.exists() { let _ = paths::rename_retry(&aside, &mods); }
        return Err(AppError::io("No se pudo volver a poner la carpeta de mods anterior", e));
    }
    Ok(aside.file_name().unwrap().to_string_lossy().to_string())
}

/// Primer nombre libre: «mods anteriores», «mods anteriores 1», «mods anteriores 2»…
pub fn previous_name(dir: &Path) -> PathBuf {
    let first = dir.join("mods anteriores");
    if !first.exists() { return first; }
    (1..10_000).map(|i| dir.join(format!("mods anteriores {i}"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("mods anteriores {}", chrono::Local::now().timestamp())))
}

/// Id de versión que el Launcher usa para el perfil.
pub fn version_id(plan: &Plan) -> AppResult<String> {
    let lv = || plan.loader_version.clone().ok_or_else(|| AppError::invalid(format!("El servidor no tiene registrada la versión de {}.", plan.loader.name())));
    Ok(match plan.loader {
        Loader::Vanilla => plan.mc_version.clone(),
        Loader::Fabric => format!("fabric-loader-{}-{}", lv()?, plan.mc_version),
        Loader::NeoForge => format!("neoforge-{}", lv()?),
        Loader::Forge => format!("{}-forge-{}", plan.mc_version, lv()?),
    })
}

fn version_json_path(mc: &Path, id: &str) -> PathBuf { mc.join("versions").join(id).join(format!("{id}.json")) }

fn check_free_space(dir: &Path, need: u64) -> AppResult<()> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let probe = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let free = disks.list().iter().filter(|d| probe.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()).map(|d| d.available_space());
    if let Some(free) = free {
        if need.saturating_add(300 * 1024 * 1024) > free {
            return Err(AppError::invalid(format!("No hay espacio suficiente en el disco: hacen falta unos {} MB libres.", need / 1_048_576 + 300)));
        }
    }
    Ok(())
}

/// Versión de Minecraft (JSON + cliente) si falta. Devuelve si la instaló.
async fn ensure_vanilla(mc: &Path, ver: &str, progress: &mut impl FnMut(&str, f64)) -> AppResult<bool> {
    let json_path = version_json_path(mc, ver);
    let jar_path = json_path.with_extension("jar");
    let v = meta::version_json(ver).await?;
    let client = v.pointer("/downloads/client").ok_or_else(|| AppError::invalid("Mojang no publica el cliente de esta versión."))?;
    let (url, sha1) = (client["url"].as_str().unwrap_or(""), client["sha1"].as_str().unwrap_or(""));
    if !url.starts_with("https://piston-data.mojang.com/") && !url.starts_with("https://launcher.mojang.com/") { return Err(AppError::invalid("La descarga no es de un servidor de Mojang.")); }
    let ok_json = json_path.is_file();
    let ok_jar = jar_path.is_file() && net::sha1_file(&jar_path).map(|h| h.eq_ignore_ascii_case(sha1)).unwrap_or(false);
    if ok_json && ok_jar { return Ok(false); }
    std::fs::create_dir_all(json_path.parent().unwrap()).map_err(|e| AppError::io("No se pudo crear la carpeta de la versión", e))?;
    if !ok_jar {
        progress(&format!("Descargando Minecraft {ver}"), 0.08);
        let tmp = jar_path.with_extension("jar.part");
        net::download(url, &tmp, net::Hash::Sha1(sha1), |d, t| { if let Some(t) = t { let _ = (d, t); } }).await?;
        paths::rename_retry(&tmp, &jar_path).map_err(|e| AppError::io("No se pudo guardar el cliente", e))?;
    }
    if !ok_json { paths::write_atomic(&json_path, serde_json::to_string_pretty(&v)?.as_bytes())?; }
    Ok(true)
}

/// Cargador para el cliente si falta. Devuelve si lo instaló.
async fn ensure_loader(mc: &Path, plan: &Plan, vid: &str, progress: &mut impl FnMut(&str, f64)) -> AppResult<bool> {
    if plan.loader == Loader::Vanilla || version_json_path(mc, vid).is_file() { return Ok(false); }
    let lv = plan.loader_version.clone().unwrap_or_default();
    match plan.loader {
        Loader::Fabric => {
            progress("Instalando Fabric", 0.3);
            let url = format!("https://meta.fabricmc.net/v2/versions/loader/{}/{}/profile/json", plan.mc_version, lv);
            let prof: serde_json::Value = net::get_json(&url, "el perfil de Fabric").await?;
            if prof["id"].as_str() != Some(vid) { return Err(AppError::invalid("Fabric devolvió un perfil de otra versión.")); }
            let p = version_json_path(mc, vid);
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| AppError::io("No se pudo crear la carpeta de Fabric", e))?;
            paths::write_atomic(&p, serde_json::to_string_pretty(&prof)?.as_bytes())?;
        }
        Loader::Forge | Loader::NeoForge => {
            let name = plan.loader.name();
            let url = meta::installer_url(plan.loader, &plan.mc_version, &lv).ok_or_else(|| AppError::invalid("Cargador sin instalador."))?;
            let sha = net::get_text(&format!("{url}.sha1"), &format!("la huella del instalador de {name}")).await?;
            let sha = sha.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
            if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(AppError::invalid(format!("El repositorio de {name} no publicó una huella válida del instalador."))); }
            let installer = paths::cache().join("installers").join(format!("{}-{}-{}-installer.jar", name.to_lowercase(), plan.mc_version, lv));
            if installer.exists() && !net::sha1_file(&installer).map(|h| h.eq_ignore_ascii_case(&sha)).unwrap_or(false) { let _ = std::fs::remove_file(&installer); }
            if !installer.exists() {
                progress(&format!("Descargando el instalador de {name}"), 0.25);
                net::download(&url, &installer, net::Hash::Sha1(&sha), |_, _| {}).await?;
            }
            let v = meta::version_json(&plan.mc_version).await?;
            let major = v.pointer("/javaVersion/majorVersion").and_then(|x| x.as_u64()).unwrap_or(21) as u32;
            progress("Preparando Java para el instalador", 0.3);
            java::ensure(major, |_, _| {}).await?;
            let java = java::installed_java(major).ok_or_else(|| AppError::new("java", "Falta Java para el instalador."))?;
            progress(&format!("Instalando {name} en el Minecraft de esta PC (puede tardar unos minutos)"), 0.35);
            let flag = if plan.loader == Loader::NeoForge { "--install-client" } else { "--installClient" };
            let (mc2, inst2) = (mc.to_path_buf(), installer.clone());
            let work = paths::cache().join("installers").join("cliente");
            std::fs::create_dir_all(&work).ok();
            let out = tokio::task::spawn_blocking(move || {
                let mut c = std::process::Command::new(java);
                crate::java::sanitize_env(&mut c);
                c.current_dir(&work).arg("-jar").arg(&inst2).arg(flag).arg(&mc2);
                #[cfg(windows)]
                { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
                c.output()
            }).await.map_err(|e| AppError::new("install", "El instalador se interrumpió").with_detail(e))?
              .map_err(|e| AppError::io("No se pudo ejecutar el instalador", e))?;
            if !out.status.success() || !version_json_path(mc, vid).is_file() {
                let mut log = out.stdout.clone(); log.extend_from_slice(&out.stderr);
                let tail: String = String::from_utf8_lossy(&log).lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
                return Err(AppError::new("install", format!("El instalador de {name} para el cliente no terminó bien.")).with_detail(tail));
            }
        }
        Loader::Vanilla => {}
    }
    Ok(true)
}

/// Agrega o actualiza el perfil en cada archivo de perfiles del Launcher, sin tocar el resto.
pub fn write_profile(mc: &Path, plan: &Plan, vid: &str, gdir: &Path, icon: Option<&str>) -> AppResult<String> {
    let files = launcher_files(mc);
    if files.is_empty() { return Err(AppError::invalid("No se encontró el Launcher de Minecraft en esta PC. Instalalo desde minecraft.net y abrilo una vez; después volvé a tocar «Preparar para jugar».")); }
    let key = format!("939host-{}", plan.server_id);
    let name = format!("939 Host · {}", plan.server_name);
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| AppError::io("No se pudo leer el archivo de perfiles del Launcher", e))?;
        let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|_| AppError::invalid("El archivo de perfiles del Launcher está dañado; abrí el Launcher una vez para que lo repare."))?;
        // Copias previas comprobadas: la original (la primera vez que 939 Host lo toca, nunca se
        // reemplaza) y la anterior a este cambio.
        let original = f.with_extension("json.original-939host");
        for b in [Some(original.clone()).filter(|o| !o.exists()), Some(f.with_extension("json.antes-939host"))].into_iter().flatten() {
            std::fs::write(&b, &text).map_err(|e| AppError::io("No se pudo hacer la copia del archivo de perfiles; no se cambió nada", e))?;
            if std::fs::read_to_string(&b).ok().as_deref() != Some(text.as_str()) { return Err(AppError::invalid("La copia del archivo de perfiles no quedó igual; no se cambió nada.")); }
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let profiles = v.as_object_mut().ok_or_else(|| AppError::invalid("Archivo de perfiles inválido."))?
            .entry("profiles").or_insert_with(|| serde_json::json!({}));
        let obj = profiles.as_object_mut().ok_or_else(|| AppError::invalid("Archivo de perfiles inválido."))?;
        let created = obj.get(&key).and_then(|p| p.get("created")).cloned().unwrap_or_else(|| serde_json::json!(now));
        let mut p = serde_json::json!({ "name": name, "type": "custom", "lastVersionId": vid, "gameDir": gdir.to_string_lossy(), "created": created, "lastUsed": now });
        p["icon"] = serde_json::json!(icon.filter(|i| i.starts_with("data:image/png;base64,") && i.len() < 200_000).unwrap_or("Grass"));
        obj.insert(key.clone(), p);
        paths::write_atomic(&f, serde_json::to_string_pretty(&v)?.as_bytes())?;
    }
    Ok(name)
}

/// Lista de Multijugador del perfil (`servers.dat`): agrega el servidor al principio si no está.
/// Los servidores que ya había quedan byte por byte como estaban (nombre, dirección, ícono, elección
/// de resource packs y todo lo demás). Si el archivo no se entiende, no se toca. Antes de cambiarlo
/// se guarda una copia (`servers.dat.antes-939host`).
pub fn add_server_entry(gdir: &Path, name: &str, address: &str) -> AppResult<()> { add_server_entry_full(gdir, name, address, None, None) }

fn write_server_entry(gdir: &Path, address: &str, entry: &[u8]) -> AppResult<()> {
    let p = gdir.join("servers.dat");
    let s = |b: &mut Vec<u8>, t: &str| { b.extend((t.len() as u16).to_be_bytes()); b.extend(t.as_bytes()); };
    let data = match std::fs::read(&p) {
        Ok(d) => d,
        Err(_) => {
            let mut b = vec![10, 0, 0, 9];
            s(&mut b, "servers");
            b.push(10);
            b.extend(1i32.to_be_bytes());
            b.extend(entry);
            b.push(0);
            return paths::write_atomic(&p, &b);
        }
    };
    if let Ok(v) = crate::nbt::parse_nbt(&data) {
        if v["servers"].as_array().into_iter().flatten().any(|e| e["ip"].as_str() == Some(address)) { return Ok(()); }
    }
    let out = nbt_insert_server(&data, entry).ok_or_else(|| AppError::invalid("La lista de servidores de ese perfil tiene un formato que 939 Host no reconoce: no se modificó. Agregá el servidor a mano desde Multijugador."))?;
    let bk = gdir.join("servers.dat.antes-939host");
    std::fs::write(&bk, &data).map_err(|e| AppError::io("No se pudo copiar la lista de servidores; no se cambió", e))?;
    paths::write_atomic(&p, &out)
}

/// Inserta un compuesto al principio de la lista `servers` de un NBT sin comprimir, sin tocar el resto.
fn nbt_insert_server(d: &[u8], entry: &[u8]) -> Option<Vec<u8>> {
    fn u16at(d: &[u8], i: usize) -> Option<usize> { Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]) as usize) }
    fn i32at(d: &[u8], i: usize) -> Option<i32> { Some(i32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?)) }
    fn skip(d: &[u8], t: u8, i: usize, depth: u32) -> Option<usize> {
        if depth > 64 { return None; }
        Some(match t {
            1 => i + 1, 2 => i + 2, 3 | 5 => i + 4, 4 | 6 => i + 8,
            7 => i + 4 + i32at(d, i)?.max(0) as usize,
            8 => i + 2 + u16at(d, i)?,
            9 => { let et = *d.get(i)?; let n = i32at(d, i + 1)?.max(0); let mut j = i + 5; for _ in 0..n { j = skip(d, et, j, depth + 1)?; } j }
            10 => { let mut j = i; loop { let tt = *d.get(j)?; j += 1; if tt == 0 { break j; } j += 2 + u16at(d, j)?; j = skip(d, tt, j, depth + 1)?; } }
            11 => i + 4 + 4 * i32at(d, i)?.max(0) as usize,
            12 => i + 4 + 8 * i32at(d, i)?.max(0) as usize,
            _ => return None,
        })
    }
    if *d.first()? != 10 { return None; }
    let mut i = 3 + u16at(d, 1)?;
    loop {
        let t = *d.get(i)?;
        if t == 0 {
            // No había lista: se agrega antes del final.
            let mut out = d[..i].to_vec();
            out.push(9); out.extend(7u16.to_be_bytes()); out.extend(b"servers"); out.push(10); out.extend(1i32.to_be_bytes()); out.extend(entry);
            out.extend(&d[i..]);
            return Some(out);
        }
        let nl = u16at(d, i + 1)?;
        let name = d.get(i + 3..i + 3 + nl)?;
        let payload = i + 3 + nl;
        if t == 9 && name == b"servers" {
            let et = *d.get(payload)?;
            let n = i32at(d, payload + 1)?;
            if et != 10 && !(et == 0 && n == 0) { return None; }
            let mut out = d[..payload].to_vec();
            out.push(10);
            out.extend((n + 1).to_be_bytes());
            out.extend(entry);
            out.extend(&d[payload + 5..]);
            return Some(out);
        }
        i = skip(d, t, payload, 0)?;
    }
}

/// Comprueba que todo lo necesario quedó en su lugar.
pub fn verify(mc: &Path, plan: &Plan, vid: &str, gdir: &Path) -> AppResult<()> {
    let vj = version_json_path(mc, vid);
    let j: serde_json::Value = serde_json::from_slice(&std::fs::read(&vj).map_err(|_| AppError::invalid(format!("Falta la versión {vid} en el Minecraft de esta PC.")))?)
        .map_err(|_| AppError::invalid(format!("La versión {vid} quedó dañada.")))?;
    if let Some(parent) = j["inheritsFrom"].as_str() {
        if !version_json_path(mc, parent).is_file() { return Err(AppError::invalid(format!("Falta Minecraft {parent}, que necesita {vid}."))); }
    }
    let mods = gdir.join("mods");
    for m in &plan.mods {
        let p = mods.join(&m.filename);
        if !net::sha1_file(&p).map(|h| h.eq_ignore_ascii_case(&m.sha1)).unwrap_or(false) { return Err(AppError::invalid(format!("El mod {} no quedó bien instalado.", m.title))); }
    }
    let key = format!("939host-{}", plan.server_id);
    for f in launcher_files(mc) {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap_or_default()).unwrap_or_default();
        if v["profiles"][&key]["lastVersionId"].as_str() != Some(vid) { return Err(AppError::invalid("El perfil no quedó guardado en el Launcher.")); }
    }
    Ok(())
}

/// Prepara todo. `fetch_manual` trae un mod agregado a mano (nombre → bytes) desde la PC que tiene
/// el servidor. `address` es la dirección con la que esta PC entra al servidor.
pub async fn prepare<F, Fut>(plan: &Plan, address: &str, icon: Option<&str>, fetch_manual: F, mut progress: impl FnMut(&str, f64)) -> AppResult<Report>
where F: Fn(String) -> Fut, Fut: std::future::Future<Output = AppResult<Vec<u8>>> {
    for m in &plan.mods {
        if m.filename.contains(['/', '\\']) || m.filename.contains("..") || !m.filename.ends_with(".jar") || m.sha1.len() != 40 { return Err(AppError::invalid("La lista de mods del servidor no es válida.")); }
        if let Some(u) = &m.url { if !u.starts_with("https://cdn.modrinth.com/") { return Err(AppError::invalid("La lista de mods del servidor tiene una descarga que no es de Modrinth.")); } }
    }
    progress("Buscando el Launcher de Minecraft", 0.02);
    let mc = minecraft_dir().ok_or_else(|| AppError::invalid("No se encontró la carpeta de Minecraft."))?;
    if launcher_files(&mc).is_empty() {
        return Err(AppError::invalid("No se encontró el Launcher de Minecraft en esta PC. Instalalo desde minecraft.net y abrilo una vez; después volvé a tocar «Preparar para jugar»."));
    }
    let vid = version_id(plan)?;
    let gdir = resolve_game_dir(&mc, plan);
    let fresh = !gdir.join(MARKER).exists() && !gdir.join("options.txt").exists();
    std::fs::create_dir_all(&gdir).map_err(|e| AppError::io("No se pudo crear la carpeta del perfil", e))?;
    // Primera vez: preferencias del último perfil usado (copia; el original no se toca).
    let mut seeded_from = None;
    let mut seeded = vec![];
    if fresh {
        if let Some((name, from)) = last_used_profile(&mc, &gdir) {
            seeded = seed_preferences(&from, &gdir)?;
            if !seeded.is_empty() { seeded_from = Some(name); }
        }
    }
    let marker = read_marker(&gdir).unwrap_or(Marker { server_id: plan.server_id.clone(), server_name: plan.server_name.clone(), seeded_from: seeded_from.clone() });
    // Perfil que ya existía sin las preferencias de la persona (por ejemplo, creado por 1.1.0): se
    // ofrece usarlas, sin hacerlo solo (ese perfil puede tener opciones propias).
    let prefs_available = if !fresh && marker.seeded_from.is_none() { last_used_profile(&mc, &gdir).map(|(n, _)| n) } else { None };
    paths::write_json(&gdir.join(MARKER), &Marker { server_name: plan.server_name.clone(), ..marker })?;
    let need: u64 = plan.mods.iter().map(|m| m.size).sum::<u64>() + if version_json_path(&mc, &plan.mc_version).is_file() { 0 } else { 60 * 1024 * 1024 };
    check_free_space(&gdir, need)?;

    let installed_version = ensure_vanilla(&mc, &plan.mc_version, &mut progress).await?;
    let installed_loader = ensure_loader(&mc, plan, &vid, &mut progress).await?;

    // Mods: primero en una carpeta aparte; se cambia por «mods» solo si todo salió bien.
    let staging = gdir.join(".mods-939host-nuevos");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| AppError::io("No se pudo crear la carpeta de mods", e))?;
    let res: AppResult<()> = async {
        let total = plan.mods.len().max(1) as f64;
        for (i, m) in plan.mods.iter().enumerate() {
            progress(&format!("Instalando mods ({} de {})", i + 1, plan.mods.len()), 0.45 + 0.4 * i as f64 / total);
            let dest = staging.join(&m.filename);
            match &m.url {
                Some(u) => net::download(u, &dest, net::Hash::Sha1(&m.sha1), |_, _| {}).await?,
                None => {
                    let b = fetch_manual(m.filename.clone()).await?;
                    if net::sha1_bytes(&b) != m.sha1.to_ascii_lowercase() { return Err(AppError::invalid(format!("El mod {} llegó distinto de como está en el servidor.", m.filename))); }
                    std::fs::write(&dest, &b).map_err(|e| AppError::io("No se pudo guardar un mod", e))?;
                }
            }
        }
        Ok(())
    }.await;
    if let Err(e) = res { let _ = std::fs::remove_dir_all(&staging); return Err(e); }

    let packs_res: AppResult<Vec<String>> = async {
    // Resource packs opcionales: se agregan a la carpeta del perfil y se activan. Nunca se pisa un
    // archivo distinto con el mismo nombre (se guarda con otro nombre).
        let mut packs = vec![];
    for pk in &plan.packs {
        if pk.filename.contains(['/', '\\']) || pk.filename.contains("..") || !pk.filename.to_ascii_lowercase().ends_with(".zip") || pk.sha1.len() != 40 { return Err(AppError::invalid("La lista de resource packs del servidor no es válida.")); }
        if let Some(u) = &pk.url { if !u.starts_with("https://cdn.modrinth.com/") { return Err(AppError::invalid("Un resource pack del servidor tiene una descarga que no es de Modrinth.")); } }
        progress(&format!("Resource pack: {}", pk.title), 0.88);
        let rp = gdir.join("resourcepacks");
        std::fs::create_dir_all(&rp).map_err(|e| AppError::io("No se pudo crear la carpeta resourcepacks", e))?;
        let mut dest = rp.join(&pk.filename);
        let same = |p: &Path| net::sha1_file(p).map(|h| h.eq_ignore_ascii_case(&pk.sha1)).unwrap_or(false);
        if dest.exists() && !same(&dest) {
            let stem = pk.filename.trim_end_matches(".zip").trim_end_matches(".ZIP");
            dest = rp.join(format!("{stem} (939 Host).zip"));
        }
        if !dest.exists() || !same(&dest) {
            let tmp = rp.join(format!(".{}.939host-descarga", pk.filename));
            match &pk.url {
                Some(u) => net::download(u, &tmp, net::Hash::Sha1(&pk.sha1), |_, _| {}).await?,
                None => {
                    let b = fetch_manual(format!("resourcepacks/{}", pk.filename)).await?;
                    if net::sha1_bytes(&b) != pk.sha1.to_ascii_lowercase() { let _ = std::fs::remove_file(&tmp); return Err(AppError::invalid(format!("El resource pack {} llegó distinto de como está en el servidor.", pk.filename))); }
                    std::fs::write(&tmp, &b).map_err(|e| AppError::io("No se pudo guardar un resource pack", e))?;
                }
            }
            paths::rename_retry(&tmp, &dest).map_err(|e| AppError::io("No se pudo guardar un resource pack", e))?;
        }
        let fname = dest.file_name().unwrap().to_string_lossy().to_string();
        enable_resource_pack(&gdir, &fname)?;
        packs.push(fname);
    }

        Ok(packs)
    }.await;
    let packs = match packs_res { Ok(p) => p, Err(e) => { let _ = std::fs::remove_dir_all(&staging); return Err(e); } };

    let mods = gdir.join("mods");
    let mut previous = None;
    let has_old = mods.is_dir() && std::fs::read_dir(&mods).map(|mut r| r.next().is_some()).unwrap_or(false);
    let same = has_old && plan.mods.len() == std::fs::read_dir(&mods).map(|r| r.count()).unwrap_or(0)
        && plan.mods.iter().all(|m| net::sha1_file(&mods.join(&m.filename)).map(|h| h.eq_ignore_ascii_case(&m.sha1)).unwrap_or(false));
    if same {
        // Ya estaba exactamente igual: nada que apartar.
        let _ = std::fs::remove_dir_all(&staging);
    } else {
        if has_old {
            let prev = previous_name(&gdir);
            paths::rename_retry(&mods, &prev).map_err(|e| { let _ = std::fs::remove_dir_all(&staging); AppError::io("No se pudo apartar la carpeta de mods anterior", e) })?;
            previous = Some(prev.file_name().unwrap().to_string_lossy().to_string());
        } else if mods.exists() {
            let _ = std::fs::remove_dir(&mods);
        }
        if let Err(e) = paths::rename_retry(&staging, &mods) {
            // Vuelve todo a como estaba.
            if let Some(p) = &previous { let _ = paths::rename_retry(&gdir.join(p), &mods); }
            let _ = std::fs::remove_dir_all(&staging);
            return Err(AppError::io("No se pudo dejar la carpeta de mods nueva", e));
        }
    }

    progress("Creando el perfil en el Launcher", 0.9);
    let mut finish = || -> AppResult<String> {
        let name = write_profile(&mc, plan, &vid, &gdir, icon)?;
        // Si la lista de servidores tiene un formato raro no se toca (el perfil igual queda listo).
        let _ = add_server_entry(&gdir, &plan.server_name, address);
        progress("Comprobando la instalación", 0.96);
        verify(&mc, plan, &vid, &gdir)?;
        Ok(name)
    };
    let name = match finish() {
        Ok(n) => n,
        Err(e) => {
            // Sin perfil válido no se deja la carpeta cambiada.
            if !same {
                let _ = std::fs::remove_dir_all(&mods);
                if let Some(p) = &previous { let _ = paths::rename_retry(&gdir.join(p), &mods); }
            }
            return Err(e);
        }
    };
    progress("Listo para jugar", 1.0);
    let label = match (&plan.loader, &plan.loader_version) {
        (Loader::Vanilla, _) | (_, None) => format!("Minecraft {}", plan.mc_version),
        (l, Some(v)) => format!("Minecraft {} con {} {}", plan.mc_version, l.name(), v),
    };
    let servers_listed = crate::nbt::read_nbt_file(&gdir.join("servers.dat")).ok().and_then(|v| v["servers"].as_array().map(|a| a.len())).unwrap_or(0);
    Ok(Report { label, profile_name: name, version_id: vid, game_dir: gdir.to_string_lossy().to_string(), mods_installed: plan.mods.len(), previous_mods: previous, server_only: plan.server_only.clone(), installed_version, installed_loader, seeded_from, seeded, servers_listed, packs, prefs_available })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_y_carpetas() {
        assert_eq!(folder_name("Supervivencia: con amigos!"), "Supervivencia- con amigos-");
        assert_eq!(folder_name("..\\..\\x"), "-..-x");
        assert_eq!(folder_name("   "), "servidor");
        let t = tempfile::tempdir().unwrap();
        assert_eq!(previous_name(t.path()).file_name().unwrap(), "mods anteriores");
        std::fs::create_dir(t.path().join("mods anteriores")).unwrap();
        assert_eq!(previous_name(t.path()).file_name().unwrap(), "mods anteriores 1");
        std::fs::create_dir(t.path().join("mods anteriores 1")).unwrap();
        assert_eq!(previous_name(t.path()).file_name().unwrap(), "mods anteriores 2");
        let p = |l: Loader, v: Option<&str>| Plan { mc_version: "1.21.1".into(), loader: l, loader_version: v.map(String::from), ..Default::default() };
        assert_eq!(version_id(&p(Loader::Vanilla, None)).unwrap(), "1.21.1");
        assert_eq!(version_id(&p(Loader::Fabric, Some("0.16.9"))).unwrap(), "fabric-loader-0.16.9-1.21.1");
        assert_eq!(version_id(&p(Loader::NeoForge, Some("21.1.77"))).unwrap(), "neoforge-21.1.77");
        assert_eq!(version_id(&p(Loader::Forge, Some("52.0.1"))).unwrap(), "1.21.1-forge-52.0.1");
        assert!(version_id(&p(Loader::Fabric, None)).is_err());
    }

    #[test]
    fn perfil_y_lista_de_servidores_sin_tocar_lo_demas() {
        let t = tempfile::tempdir().unwrap();
        let mc = t.path();
        let plan = Plan { server_id: "srv1".into(), server_name: "Mi server".into(), mc_version: "1.21.1".into(), ..Default::default() };
        let g = game_dir(mc, &plan);
        assert!(write_profile(mc, &plan, "1.21.1", &g, None).is_err(), "sin Launcher avisa");
        std::fs::write(mc.join("launcher_profiles.json"), r#"{"profiles":{"abc":{"name":"Mío","type":"custom","lastVersionId":"1.20.1"}},"settings":{"x":1},"version":3}"#).unwrap();
        let name = write_profile(mc, &plan, "1.21.1", &g, None).unwrap();
        assert_eq!(name, "939 Host · Mi server");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(mc.join("launcher_profiles.json")).unwrap()).unwrap();
        assert_eq!(v["profiles"]["abc"]["name"], "Mío", "el perfil del usuario sigue igual");
        assert_eq!(v["settings"]["x"], 1);
        assert_eq!(v["profiles"]["939host-srv1"]["lastVersionId"], "1.21.1");
        assert!(mc.join("launcher_profiles.json.antes-939host").is_file());
        std::fs::create_dir_all(&g).unwrap();
        add_server_entry(&g, "Mi server", "127.0.0.1:25600").unwrap();
        add_server_entry(&g, "Mi server", "127.0.0.1:25600").unwrap();
        let s = crate::nbt::read_nbt_file(&g.join("servers.dat")).unwrap();
        assert_eq!(s["servers"].as_array().unwrap().len(), 1);
        assert_eq!(s["servers"][0]["ip"], "127.0.0.1:25600");
        // Verificación: versión que falta → error claro.
        assert!(verify(mc, &plan, "1.21.1", &g).unwrap_err().message.contains("Falta la versión"));
        std::fs::create_dir_all(mc.join("versions/1.21.1")).unwrap();
        std::fs::write(mc.join("versions/1.21.1/1.21.1.json"), "{}").unwrap();
        verify(mc, &plan, "1.21.1", &g).unwrap();
    }

    /// servers.dat con dos servidores, uno con ícono y «aceptar packs», escrito a mano (como Minecraft).
    fn servers_dat_real() -> Vec<u8> {
        let s = |b: &mut Vec<u8>, t: &str| { b.extend((t.len() as u16).to_be_bytes()); b.extend(t.as_bytes()); };
        let mut b = vec![10, 0, 0, 9];
        s(&mut b, "servers");
        b.push(10);
        b.extend(2i32.to_be_bytes());
        b.push(8); s(&mut b, "ip"); s(&mut b, "mc.hypixel.net");
        b.push(8); s(&mut b, "name"); s(&mut b, "Hypixel");
        b.push(8); s(&mut b, "icon"); s(&mut b, "iVBORw0KGgoAAAANSUhEUg==");
        b.push(1); s(&mut b, "acceptTextures"); b.push(1);
        b.push(0);
        b.push(8); s(&mut b, "ip"); s(&mut b, "192.168.1.5:25565");
        b.push(8); s(&mut b, "name"); s(&mut b, "Casa");
        b.push(0);
        b.push(0);
        b
    }

    #[test]
    fn lista_de_servidores_conserva_todo_lo_que_habia() {
        let t = tempfile::tempdir().unwrap();
        let orig = servers_dat_real();
        std::fs::write(t.path().join("servers.dat"), &orig).unwrap();
        add_server_entry(t.path(), "939 Host · Mi server", "127.0.0.1:25600").unwrap();
        let after = std::fs::read(t.path().join("servers.dat")).unwrap();
        let v = crate::nbt::parse_nbt(&after).unwrap();
        let list = v["servers"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0]["ip"], "127.0.0.1:25600");
        assert_eq!(list[1]["name"], "Hypixel");
        assert_eq!(list[1]["icon"], "iVBORw0KGgoAAAANSUhEUg==", "el ícono se conserva");
        assert_eq!(list[1]["acceptTextures"], 1, "la elección de packs se conserva");
        assert_eq!(list[2]["ip"], "192.168.1.5:25565");
        // Los servidores que había quedan byte por byte.
        let tail = &orig[orig.len() - 60..];
        assert!(after.windows(tail.len()).any(|w| w == tail));
        assert_eq!(std::fs::read(t.path().join("servers.dat.antes-939host")).unwrap(), orig, "copia previa idéntica");
        // Repetir no duplica.
        add_server_entry(t.path(), "939 Host · Mi server", "127.0.0.1:25600").unwrap();
        assert_eq!(crate::nbt::read_nbt_file(&t.path().join("servers.dat")).unwrap()["servers"].as_array().unwrap().len(), 3);
        // Un archivo que no se entiende no se toca.
        std::fs::write(t.path().join("servers.dat"), b"\x0anada").unwrap();
        assert!(add_server_entry(t.path(), "x", "1.2.3.4").is_err());
        assert_eq!(std::fs::read(t.path().join("servers.dat")).unwrap(), b"\x0anada");
    }

    #[test]
    fn perfil_nuevo_toma_las_preferencias_del_ultimo_usado_sin_tocarlo() {
        let t = tempfile::tempdir().unwrap();
        let mc = t.path();
        let mine = mc.join("perfiles/mio");
        let old = mc.join("perfiles/viejo");
        for d in [&mine, &old] { std::fs::create_dir_all(d).unwrap(); }
        std::fs::write(mine.join("options.txt"), "lang:es_ar\nkey_key.jump:key.keyboard.g\nnarrator:2\nhighContrast:true\n").unwrap();
        std::fs::write(mine.join("servers.dat"), servers_dat_real()).unwrap();
        std::fs::write(old.join("options.txt"), "lang:en_us\n").unwrap();
        std::fs::write(mc.join("launcher_profiles.json"), serde_json::json!({"profiles": {
            "a": {"name": "Mío", "lastVersionId": "1.21.1", "gameDir": mine, "lastUsed": "2026-11-20T10:00:00.000Z"},
            "b": {"name": "Viejo", "lastVersionId": "1.20.1", "gameDir": old, "lastUsed": "2025-01-01T10:00:00.000Z"}}}).to_string()).unwrap();
        let plan = Plan { server_id: "s1".into(), server_name: "Server".into(), ..Default::default() };
        let g = resolve_game_dir(mc, &plan);
        std::fs::create_dir_all(&g).unwrap();
        let (name, from) = last_used_profile(mc, &g).unwrap();
        assert_eq!(name, "Mío");
        let done = seed_preferences(&from, &g).unwrap();
        assert_eq!(done, vec!["options.txt", "servers.dat"]);
        assert_eq!(std::fs::read(g.join("options.txt")).unwrap(), std::fs::read(mine.join("options.txt")).unwrap());
        // Lo que ya existe en el perfil no se pisa.
        std::fs::write(g.join("options.txt"), "lang:pt_br\n").unwrap();
        seed_preferences(&from, &g).unwrap();
        assert_eq!(std::fs::read_to_string(g.join("options.txt")).unwrap(), "lang:pt_br\n");
        assert_eq!(std::fs::read_to_string(mine.join("options.txt")).unwrap(), "lang:es_ar\nkey_key.jump:key.keyboard.g\nnarrator:2\nhighContrast:true\n", "el original no cambia");
        enable_resource_pack(&g, "pack opcional.zip").unwrap();
        enable_resource_pack(&g, "pack opcional.zip").unwrap();
        assert_eq!(std::fs::read_to_string(g.join("options.txt")).unwrap(), "lang:pt_br\nresourcePacks:[\"vanilla\",\"file/pack opcional.zip\"]\n");
    }

    #[test]
    fn carpetas_de_servidores_con_el_mismo_nombre_no_se_mezclan_y_se_respeta_la_del_launcher() {
        let t = tempfile::tempdir().unwrap();
        let mc = t.path();
        std::fs::write(mc.join("launcher_profiles.json"), r#"{"profiles":{}}"#).unwrap();
        let a = Plan { server_id: "a".into(), server_name: "Supervivencia".into(), ..Default::default() };
        let b = Plan { server_id: "b".into(), server_name: "Supervivencia".into(), ..Default::default() };
        let ga = resolve_game_dir(mc, &a);
        std::fs::create_dir_all(&ga).unwrap();
        paths::write_json(&ga.join(MARKER), &Marker { server_id: "a".into(), server_name: "Supervivencia".into(), seeded_from: None }).unwrap();
        let gb = resolve_game_dir(mc, &b);
        assert_ne!(ga, gb, "otro servidor con el mismo nombre usa otra carpeta");
        assert_eq!(resolve_game_dir(mc, &a), ga);
        // Si la persona cambió la carpeta del perfil en el Launcher, se usa esa.
        let custom = mc.join("mis juegos/super");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(mc.join("launcher_profiles.json"), serde_json::json!({"profiles": {"939host-a": {"name": "939 Host · Supervivencia", "gameDir": custom}}}).to_string()).unwrap();
        assert_eq!(resolve_game_dir(mc, &a), custom);
    }

    #[test]
    fn deshacer_el_cambio_de_mods_no_borra_nada() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("mods anteriores")).unwrap();
        std::fs::write(g.join("mods anteriores/mio.jar"), b"m").unwrap();
        std::fs::create_dir_all(g.join("mods")).unwrap();
        std::fs::write(g.join("mods/939.jar"), b"n").unwrap();
        let aside = undo_mods(g, "mods anteriores").unwrap();
        assert!(g.join("mods/mio.jar").is_file());
        assert!(g.join(&aside).join("939.jar").is_file());
        assert!(undo_mods(g, "../x").is_err());
    }

    #[test]
    fn usar_preferencias_en_un_perfil_existente_guarda_las_actuales() {
        let t = tempfile::tempdir().unwrap();
        let (from, g) = (t.path().join("mio"), t.path().join("939"));
        for d in [&from, &g] { std::fs::create_dir_all(d).unwrap(); }
        std::fs::write(from.join("options.txt"), "lang:es_ar\n").unwrap();
        std::fs::write(from.join("servers.dat"), servers_dat_real()).unwrap();
        std::fs::write(g.join("options.txt"), "lang:en_us\n").unwrap();
        add_server_entry(&g, "939 Host · X", "127.0.0.1:25600").unwrap();
        let done = import_preferences(&from, &g).unwrap();
        assert!(done.contains(&"options.txt".to_string()));
        assert_eq!(std::fs::read_to_string(g.join("options.txt")).unwrap(), "lang:es_ar\n");
        assert_eq!(std::fs::read_to_string(g.join("options.txt.antes-939host")).unwrap(), "lang:en_us\n", "las opciones que había quedan guardadas");
        let v = crate::nbt::read_nbt_file(&g.join("servers.dat")).unwrap();
        let ips: Vec<&str> = v["servers"].as_array().unwrap().iter().map(|e| e["ip"].as_str().unwrap()).collect();
        assert!(ips.contains(&"127.0.0.1:25600") && ips.contains(&"mc.hypixel.net") && ips.contains(&"192.168.1.5:25565"));
        let hyp = v["servers"].as_array().unwrap().iter().find(|e| e["ip"] == "mc.hypixel.net").unwrap();
        assert_eq!(hyp["icon"], "iVBORw0KGgoAAAANSUhEUg==");
        // Otra vez: nada que cambiar, ninguna copia nueva.
        assert!(import_preferences(&from, &g).unwrap().is_empty());
        assert!(!g.join("options.txt.antes-939host-2").exists());
    }
}
