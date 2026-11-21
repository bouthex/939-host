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
//! 4. Usa una carpeta de juego propia por servidor (`.minecraft\939 Host\<servidor>`): los mods de
//!    otros perfiles nunca se mezclan. Si esa carpeta ya tenía `mods`, se renombra a
//!    «mods anteriores» (o «mods anteriores 1», «2», …) y se crea una nueva.
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
    pub profile_name: String,
    pub version_id: String,
    pub game_dir: String,
    pub mods_installed: usize,
    pub previous_mods: Option<String>,
    pub server_only: Vec<String>,
    pub installed_version: bool,
    pub installed_loader: bool,
}

/// Carpeta del Minecraft de esta PC. `H939_MINECRAFT_DIR` solo para pruebas (nunca la real).
pub fn minecraft_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("H939_MINECRAFT_DIR") { return Some(PathBuf::from(d)); }
    #[cfg(windows)]
    { std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(".minecraft")) }
    #[cfg(not(windows))]
    { std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".minecraft")) }
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

/// Instancia del servidor dentro de `.minecraft`.
pub fn game_dir(mc: &Path, plan: &Plan) -> PathBuf { mc.join("939 Host").join(folder_name(&plan.server_name)) }

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
        // Copia previa (una sola, la más reciente).
        let _ = std::fs::write(f.with_extension("json.antes-939host"), &text);
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

/// Lista de Multijugador del perfil (`servers.dat`): agrega el servidor si no está.
pub fn add_server_entry(gdir: &Path, name: &str, address: &str) -> AppResult<()> {
    let p = gdir.join("servers.dat");
    let mut entries: Vec<(String, String)> = vec![];
    if let Ok(v) = crate::nbt::read_nbt_file(&p) {
        for s in v["servers"].as_array().into_iter().flatten() {
            if let (Some(n), Some(ip)) = (s["name"].as_str(), s["ip"].as_str()) { entries.push((n.into(), ip.into())); }
        }
    }
    if entries.iter().any(|(_, ip)| ip == address) { return Ok(()); }
    entries.insert(0, (name.to_string(), address.to_string()));
    // NBT sin comprimir: { servers: [ { name, ip } … ] }
    let s = |b: &mut Vec<u8>, t: &str| { b.extend((t.len() as u16).to_be_bytes()); b.extend(t.as_bytes()); };
    let mut b = vec![10, 0, 0, 9];
    s(&mut b, "servers");
    b.push(10);
    b.extend((entries.len() as i32).to_be_bytes());
    for (n, ip) in &entries { b.push(8); s(&mut b, "name"); s(&mut b, n); b.push(8); s(&mut b, "ip"); s(&mut b, ip); b.push(0); }
    b.push(0);
    paths::write_atomic(&p, &b)
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
    let gdir = game_dir(&mc, plan);
    std::fs::create_dir_all(&gdir).map_err(|e| AppError::io("No se pudo crear la carpeta del perfil", e))?;
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
    Ok(Report { profile_name: name, version_id: vid, game_dir: gdir.to_string_lossy().to_string(), mods_installed: plan.mods.len(), previous_mods: previous, server_only: plan.server_only.clone(), installed_version, installed_loader })
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
}
