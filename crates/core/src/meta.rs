//! Metadatos oficiales: versiones de Mojang y de Fabric. No se inventa ninguna compatibilidad.

use crate::error::{AppError, AppResult};
use crate::model::Loader;
use crate::net;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

const MOJANG_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_META: &str = "https://meta.fabricmc.net/v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionOption {
    pub id: String,
    pub kind: String,
    pub release_time: String,
}

/// Manifiesto con caché de 1 hora para no consultar a Mojang en cada apertura.
async fn manifest() -> AppResult<Manifest> {
    let cache = paths::cache().join("version_manifest_v2.json");
    if let Ok(meta) = std::fs::metadata(&cache) {
        let fresh = meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).map(|age| age < Duration::from_secs(3600)).unwrap_or(false);
        if fresh {
            if let Ok(m) = paths::read_json::<Manifest>(&cache) { return Ok(m); }
        }
    }
    match net::get_json::<serde_json::Value>(MOJANG_MANIFEST, "el listado de versiones de Mojang").await {
        Ok(value) => {
            let _ = paths::write_json(&cache, &value);
            Ok(serde_json::from_value(value)?)
        }
        // Sin internet: usar la caché vieja si existe.
        Err(e) => paths::read_json::<Manifest>(&cache).map_err(|_| e),
    }
}

pub async fn list_versions(include_snapshots: bool) -> AppResult<Vec<VersionOption>> {
    let m = manifest().await?;
    Ok(m.versions
        .into_iter()
        .filter(|v| v.kind == "release" || (include_snapshots && v.kind == "snapshot"))
        .map(|v| VersionOption { id: v.id, kind: v.kind, release_time: v.release_time })
        .collect())
}

#[derive(Debug, Clone)]
pub struct ServerDownload {
    pub url: String,
    pub sha1: String,
    pub java_major: u32,
}

pub async fn server_download(mc_version: &str) -> AppResult<ServerDownload> {
    let m = manifest().await?;
    let entry = m.versions.iter().find(|v| v.id == mc_version).ok_or_else(|| AppError::not_found(&format!("la versión {mc_version} en el listado de Mojang")))?;
    let v: serde_json::Value = net::get_json(&entry.url, "los datos de la versión").await?;
    let server = v.pointer("/downloads/server").ok_or_else(|| AppError::invalid(format!("Mojang no publica servidor dedicado para la versión {mc_version}.")))?;
    let url = server.get("url").and_then(|x| x.as_str()).ok_or_else(|| AppError::invalid("Datos de versión incompletos"))?;
    let sha1 = server.get("sha1").and_then(|x| x.as_str()).ok_or_else(|| AppError::invalid("Datos de versión incompletos"))?;
    let java_major = v.pointer("/javaVersion/majorVersion").and_then(|x| x.as_u64()).unwrap_or(21) as u32;
    Ok(ServerDownload { url: url.to_string(), sha1: sha1.to_string(), java_major })
}

#[derive(Debug, Deserialize)]
struct FabricGame { version: String }
#[derive(Debug, Deserialize)]
struct FabricLoaderEntry { loader: FabricLoaderInfo }
#[derive(Debug, Deserialize)]
struct FabricLoaderInfo { version: String, stable: bool }
#[derive(Debug, Deserialize)]
struct FabricInstaller { version: String, stable: bool }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FabricSupport {
    pub supported: bool,
    pub loader_version: Option<String>,
}

pub async fn fabric_support(mc_version: &str) -> AppResult<FabricSupport> {
    let games: Vec<FabricGame> = net::get_json(&format!("{FABRIC_META}/versions/game"), "el listado de Fabric").await?;
    if !games.iter().any(|g| g.version == mc_version) {
        return Ok(FabricSupport { supported: false, loader_version: None });
    }
    let loaders: Vec<FabricLoaderEntry> = net::get_json(&format!("{FABRIC_META}/versions/loader/{mc_version}"), "los loaders de Fabric").await?;
    let pick = loaders.iter().find(|l| l.loader.stable).or_else(|| loaders.first());
    Ok(FabricSupport { supported: pick.is_some(), loader_version: pick.map(|l| l.loader.version.clone()) })
}

/// URL del lanzador de servidor de Fabric (descarga las librerías de Fabric en el primer arranque).
pub async fn fabric_server_launcher_url(mc_version: &str, loader: &str) -> AppResult<String> {
    let installers: Vec<FabricInstaller> = net::get_json(&format!("{FABRIC_META}/versions/installer"), "el instalador de Fabric").await?;
    let inst = installers.iter().find(|i| i.stable).or_else(|| installers.first()).ok_or_else(|| AppError::not_found("un instalador de Fabric"))?;
    Ok(format!("{FABRIC_META}/versions/loader/{mc_version}/{loader}/{}/server/jar", inst.version))
}

/* ───────── Forge y NeoForge ───────── */

const FORGE_PROMOS: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_VERSIONS: &str = "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

#[derive(Deserialize)]
struct ForgePromos { promos: std::collections::HashMap<String, String> }

#[derive(Deserialize)]
struct NeoVersions { versions: Vec<String> }

/// Versión de Forge para una versión de Minecraft: la recomendada, o la última.
pub fn pick_forge(promos: &std::collections::HashMap<String, String>, mc: &str) -> Option<String> {
    promos.get(&format!("{mc}-recommended")).or_else(|| promos.get(&format!("{mc}-latest"))).cloned()
}

/// Prefijo de versión de NeoForge para una versión de Minecraft.
/// 1.21.1 → "21.1."; 1.21 → "21.0."; 26.1 → "26.1.0."; 26.1.2 → "26.1.2.".
pub fn neoforge_prefix(mc: &str) -> Option<String> {
    let parts: Vec<&str> = mc.split('.').collect();
    if parts.iter().any(|p| p.parse::<u32>().is_err()) { return None; }
    if parts.first() == Some(&"1") {
        if parts.len() < 2 { return None; }
        let minor: u32 = parts[1].parse().ok()?;
        let patch: u32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
        // NeoForge empieza en 1.20.2 (para 1.20.1 se usaba otro artefacto).
        if minor < 20 || (minor == 20 && patch < 2) { return None; }
        Some(format!("{}.{}.", parts[1], parts.get(2).copied().unwrap_or("0")))
    } else {
        Some(format!("{}.{}.{}.", parts[0], parts.get(1).copied().unwrap_or("0"), parts.get(2).copied().unwrap_or("0")))
    }
}

/// Elige la versión de NeoForge: la última estable con ese prefijo; si no hay estables, la última beta.
pub fn pick_neoforge(versions: &[String], mc: &str) -> Option<String> {
    let prefix = neoforge_prefix(mc)?;
    let matching: Vec<&String> = versions.iter().filter(|v| v.starts_with(&prefix)).collect();
    let key = |v: &str| -> Vec<u64> { v.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(|x| x.parse().unwrap_or(0)).collect() };
    let best = |list: Vec<&String>| list.into_iter().max_by(|a, b| key(a).cmp(&key(b))).cloned();
    best(matching.iter().copied().filter(|v| !v.contains("beta") && !v.contains("alpha")).collect()).or_else(|| best(matching))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoaderSupport {
    pub supported: bool,
    pub loader_version: Option<String>,
    /// La descarga del loader no se pudo probar en el entorno de desarrollo de 939 Host.
    pub unverified: bool,
}

pub async fn loader_support(loader: Loader, mc: &str) -> AppResult<LoaderSupport> {
    match loader {
        Loader::Vanilla => Ok(LoaderSupport { supported: true, loader_version: None, unverified: false }),
        Loader::Fabric => fabric_support(mc).await.map(|f| LoaderSupport { supported: f.supported, loader_version: f.loader_version, unverified: false }),
        Loader::Forge => {
            let p: ForgePromos = net::get_json(FORGE_PROMOS, "las versiones de Forge").await?;
            let v = pick_forge(&p.promos, mc);
            Ok(LoaderSupport { supported: v.is_some(), loader_version: v, unverified: true })
        }
        Loader::NeoForge => {
            let p: NeoVersions = net::get_json(NEOFORGE_VERSIONS, "las versiones de NeoForge").await?;
            let v = pick_neoforge(&p.versions, mc);
            Ok(LoaderSupport { supported: v.is_some(), loader_version: v, unverified: true })
        }
    }
}

/// URL del instalador oficial (Forge o NeoForge).
pub fn installer_url(loader: Loader, mc: &str, version: &str) -> Option<String> {
    match loader {
        Loader::Forge => Some(format!("{FORGE_MAVEN}/{mc}-{version}/forge-{mc}-{version}-installer.jar")),
        Loader::NeoForge => Some(format!("{NEOFORGE_MAVEN}/{version}/neoforge-{version}-installer.jar")),
        _ => None,
    }
}

#[cfg(test)]
mod loader_tests {
    use super::*;

    #[test]
    fn forge_and_neoforge_version_selection() {
        let mut promos = std::collections::HashMap::new();
        promos.insert("1.20.1-recommended".to_string(), "47.3.0".to_string());
        promos.insert("1.20.1-latest".to_string(), "47.4.10".to_string());
        promos.insert("1.21.4-latest".to_string(), "54.1.0".to_string());
        assert_eq!(pick_forge(&promos, "1.20.1").as_deref(), Some("47.3.0"));
        assert_eq!(pick_forge(&promos, "1.21.4").as_deref(), Some("54.1.0"));
        assert_eq!(pick_forge(&promos, "26.2"), None);

        assert_eq!(neoforge_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("1.20.4").as_deref(), Some("20.4."));
        assert_eq!(neoforge_prefix("1.20.1"), None);
        assert_eq!(neoforge_prefix("26.1").as_deref(), Some("26.1.0."));
        assert_eq!(neoforge_prefix("26.1.2").as_deref(), Some("26.1.2."));
        let vs: Vec<String> = ["21.1.9", "21.1.77", "21.1.100-beta", "21.10.3", "26.1.0.1-beta", "26.1.0.4-beta"].iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_neoforge(&vs, "1.21.1").as_deref(), Some("21.1.77"));
        assert_eq!(pick_neoforge(&vs, "1.21.10").as_deref(), Some("21.10.3"));
        assert_eq!(pick_neoforge(&vs, "26.1").as_deref(), Some("26.1.0.4-beta"));
        assert_eq!(pick_neoforge(&vs, "1.19.2"), None);
        assert!(installer_url(Loader::NeoForge, "1.21.1", "21.1.77").unwrap().ends_with("/21.1.77/neoforge-21.1.77-installer.jar"));
        assert!(installer_url(Loader::Forge, "1.20.1", "47.3.0").unwrap().ends_with("/1.20.1-47.3.0/forge-1.20.1-47.3.0-installer.jar"));
    }
}

/* ───────── Fabric sin meta.fabricmc.net (plan B desde el repositorio Maven oficial) ───────── */

const FABRIC_MAVEN: &str = "https://maven.fabricmc.net";

/// "net.fabricmc:fabric-loader:0.20.1" → "net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar"
pub fn maven_path(coord: &str) -> Option<String> {
    let p: Vec<&str> = coord.split(':').collect();
    if p.len() != 3 || p.iter().any(|x| x.is_empty() || x.contains("..") || x.contains('/')) { return None; }
    Some(format!("{}/{}/{}/{}-{}.jar", p[0].replace('.', "/"), p[1], p[2], p[1], p[2]))
}

/// Última versión estable del loader según el maven-metadata.xml.
pub fn latest_from_metadata(xml: &str) -> Option<String> {
    let versions: Vec<&str> = xml.split("<version>").skip(1).filter_map(|x| x.split("</version>").next()).collect();
    versions.into_iter().filter(|v| !v.contains('+') && !v.contains("beta") && !v.contains("pre")).last().map(String::from)
}

/// Versiones de Minecraft que no necesitan mapeos intermedios (sin ofuscación: 26.1 en adelante).
pub fn needs_intermediary(mc: &str) -> bool { mc.starts_with("1.") }

#[derive(Deserialize)]
struct LoaderJson { libraries: LoaderLibs }
#[derive(Deserialize)]
struct LoaderLibs { #[serde(default)] common: Vec<MavenLib>, #[serde(default)] server: Vec<MavenLib> }
#[derive(Deserialize)]
struct MavenLib { name: String, #[serde(default)] url: Option<String> }

/// Compatibilidad sin meta: el loader más reciente y, si hace falta, que existan los mapeos.
pub async fn fabric_support_maven(mc: &str) -> AppResult<FabricSupport> {
    let xml = net::get_text(&format!("{FABRIC_MAVEN}/net/fabricmc/fabric-loader/maven-metadata.xml"), "el repositorio de Fabric").await?;
    let v = latest_from_metadata(&xml);
    if needs_intermediary(mc) {
        let xml = net::get_text(&format!("{FABRIC_MAVEN}/net/fabricmc/intermediary/maven-metadata.xml"), "el repositorio de Fabric").await?;
        if !xml.contains(&format!("<version>{mc}</version>")) { return Ok(FabricSupport { supported: false, loader_version: None }); }
    }
    Ok(FabricSupport { supported: v.is_some(), loader_version: v })
}

/// Instala Fabric descargando sus librerías del Maven oficial (cada una verificada con su .sha1)
/// y deja un archivo de argumentos para arrancar. Devuelve el nombre del archivo de argumentos.
pub async fn fabric_install_maven(run: &std::path::Path, mc: &str, loader: &str) -> AppResult<String> {
    let lj: LoaderJson = net::get_json(&format!("{FABRIC_MAVEN}/net/fabricmc/fabric-loader/{loader}/fabric-loader-{loader}.json"), "el loader de Fabric").await?;
    let mut libs: Vec<(String, String)> = lj.libraries.common.into_iter().chain(lj.libraries.server)
        .map(|l| (l.name, l.url.unwrap_or_else(|| format!("{FABRIC_MAVEN}/")))).collect();
    libs.push((format!("net.fabricmc:fabric-loader:{loader}"), format!("{FABRIC_MAVEN}/")));
    if needs_intermediary(mc) { libs.push((format!("net.fabricmc:intermediary:{mc}"), format!("{FABRIC_MAVEN}/"))); }
    let mut cp = vec![];
    for (coord, base) in libs {
        if !base.starts_with("https://") { return Err(AppError::invalid("Origen de librería no seguro.")); }
        let rel = maven_path(&coord).ok_or_else(|| AppError::invalid(format!("Coordenada inválida: {coord}")))?;
        let url = format!("{}/{}", base.trim_end_matches('/'), rel);
        let sha = net::get_text(&format!("{url}.sha1"), "la huella de una librería").await?;
        let sha = sha.split_whitespace().next().unwrap_or("").to_string();
        let dest = run.join("libraries").join(&rel);
        if !(dest.exists() && net::sha1_file(&dest).map(|h| h == sha).unwrap_or(false)) {
            net::download(&url, &dest, if sha.len() == 40 { net::Hash::Sha1(&sha) } else { net::Hash::None }, |_, _| {}).await?;
        }
        cp.push(format!("libraries/{rel}"));
    }
    let sep = if cfg!(windows) { ";" } else { ":" };
    let args = format!("-Dfabric.gameJarPath=server.jar\n-cp\n\"{}\"\nnet.fabricmc.loader.impl.launch.knot.KnotServer\n", cp.join(sep));
    let name = "939host-fabric-args.txt".to_string();
    paths::write_atomic(&run.join(&name), args.as_bytes())?;
    Ok(name)
}

#[cfg(test)]
mod fabric_maven_tests {
    use super::*;
    #[test]
    fn maven_helpers() {
        assert_eq!(maven_path("net.fabricmc:fabric-loader:0.20.1").unwrap(), "net/fabricmc/fabric-loader/0.20.1/fabric-loader-0.20.1.jar");
        assert_eq!(maven_path("org.ow2.asm:asm:9.10.1").unwrap(), "org/ow2/asm/asm/9.10.1/asm-9.10.1.jar");
        assert!(maven_path("a:../b:1").is_none());
        let xml = "<versions><version>0.19.5</version><version>0.20.0+build.1</version><version>0.20.1</version></versions>";
        assert_eq!(latest_from_metadata(xml).as_deref(), Some("0.20.1"));
        assert!(!needs_intermediary("26.2"));
        assert!(needs_intermediary("1.21.1"));
    }
}
