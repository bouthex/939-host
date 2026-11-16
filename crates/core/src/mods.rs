//! Mods de un servidor: plan de instalación con dependencias, instalación verificada,
//! desinstalación segura, actualizaciones y diagnóstico de la carpeta `mods`.

use crate::error::{AppError, AppResult};
use crate::model::{Loader, ServerConfig};
use crate::modrinth::{self, DependencyType, Project, Source, Version};
use crate::net;
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMod {
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub version_number: String,
    pub file_name: String,
    pub sha1: String,
    pub side: String,
    /// true = lo eligió el usuario; false = se instaló como dependencia.
    pub explicit: bool,
    #[serde(default)]
    pub required_by: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub mods: Vec<InstalledMod>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub project_id: String,
    pub title: String,
    pub version: Version,
    pub side: String,
    pub reason: String,
    pub explicit: bool,
    pub depends_on: Vec<String>,
    pub replaces: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub items: Vec<PlanItem>,
    pub warnings: Vec<String>,
    pub total_bytes: u64,
}

pub fn loader_id(l: Loader) -> AppResult<&'static str> {
    match l {
        Loader::Fabric => Ok("fabric"),
        Loader::Forge => Ok("forge"),
        Loader::NeoForge => Ok("neoforge"),
        Loader::Vanilla => Err(AppError::invalid("Los servidores Vanilla no usan mods. Creá un servidor Fabric, Forge o NeoForge para instalar mods.")),
    }
}

fn manifest_path(id: &str) -> PathBuf { paths::server_dir(id).join("mods.json") }
pub fn mods_dir(id: &str) -> PathBuf { paths::server_run_dir(id).join("mods") }

pub fn load_manifest(id: &str) -> Manifest { paths::read_json(&manifest_path(id)).unwrap_or_default() }
pub fn save_manifest(id: &str, m: &Manifest) -> AppResult<()> { paths::write_json(&manifest_path(id), m) }

/// Arma el plan para instalar `version_id` (o la mejor versión del proyecto) con sus
/// dependencias obligatorias, verificando incompatibilidades y el lado (cliente/servidor).
pub async fn plan_install<S: Source>(src: &S, cfg: &ServerConfig, project_id: &str, version_id: Option<&str>) -> AppResult<InstallPlan> {
    let loader = loader_id(cfg.loader)?;
    let mc = cfg.mc_version.as_str();
    let manifest = load_manifest(&cfg.id);
    let installed: HashMap<&str, &InstalledMod> = manifest.mods.iter().map(|m| (m.project_id.as_str(), m)).collect();

    let root = match version_id {
        Some(v) => src.version(v).await?,
        None => modrinth::best_version(src.project_versions(project_id, mc, loader).await?, mc, loader)
            .ok_or_else(|| AppError::invalid(format!("Este mod no tiene una versión para {} {mc}.", loader_title(loader))))?,
    };
    if !root.supports(mc, loader) {
        return Err(AppError::invalid(format!("La versión {} no es compatible con {} {mc}.", root.version_number, loader_title(loader))));
    }

    let mut items: Vec<PlanItem> = vec![];
    let mut warnings = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(Version, String, bool)> = VecDeque::new();
    queue.push_back((root, "Elegido por vos".into(), true));

    while let Some((version, reason, explicit)) = queue.pop_front() {
        if !seen.insert(version.project_id.clone()) { continue; }
        let mut depends_on = vec![];
        for dep in &version.dependencies {
            let Some(pid) = dep.project_id.clone().or(None) else {
                if dep.dependency_type == DependencyType::Required {
                    if let Some(vid) = &dep.version_id {
                        let v = src.version(vid).await?;
                        depends_on.push(v.project_id.clone());
                        if !installed.contains_key(v.project_id.as_str()) { queue.push_back((v, format!("Necesario para {}", version.name), false)); }
                    }
                }
                continue;
            };
            match dep.dependency_type {
                DependencyType::Required => {
                    depends_on.push(pid.clone());
                    if installed.contains_key(pid.as_str()) || seen.contains(&pid) { continue; }
                    let dv = match &dep.version_id {
                        Some(vid) => src.version(vid).await?,
                        None => modrinth::best_version(src.project_versions(&pid, mc, loader).await?, mc, loader)
                            .ok_or_else(|| AppError::invalid(format!("Falta una dependencia obligatoria sin versión para {mc}: {pid}. No se puede instalar sin ella.")))?,
                    };
                    if !dv.supports(mc, loader) {
                        return Err(AppError::invalid(format!("La dependencia {} no es compatible con {} {mc}.", dv.name, loader_title(loader))));
                    }
                    queue.push_back((dv, format!("Necesario para {}", version.name), false));
                }
                DependencyType::Incompatible => {
                    if let Some(m) = installed.get(pid.as_str()) {
                        return Err(AppError::invalid(format!("{} es incompatible con «{}», que ya está instalado. Quitalo primero si querés cambiarlo.", version.name, m.title)));
                    }
                }
                DependencyType::Optional => {}
                DependencyType::Embedded => {}
            }
        }
        let replaces = installed.get(version.project_id.as_str()).map(|m| m.version_number.clone());
        items.push(PlanItem { project_id: version.project_id.clone(), title: version.name.clone(), side: "both".into(), reason, explicit, depends_on, replaces, version });
    }

    // Lados (cliente/servidor) según el proyecto.
    let ids: Vec<String> = items.iter().map(|i| i.project_id.clone()).collect();
    let projects: HashMap<String, Project> = src.projects(&ids).await?.into_iter().map(|p| (p.id.clone(), p)).collect();
    for item in &mut items {
        if let Some(p) = projects.get(&item.project_id) {
            item.title = p.title.clone();
            item.side = modrinth::side_label(&p.client_side, &p.server_side).into();
            if !modrinth::runs_on_server(&p.server_side) {
                if item.explicit {
                    return Err(AppError::invalid(format!("«{}» es un mod solo de cliente: no funciona en un servidor dedicado. Va en el perfil de cada jugador.", p.title)));
                }
                warnings.push(format!("«{}» es solo de cliente; se omite en el servidor.", p.title));
            }
        }
    }
    items.retain(|i| i.side != "client");
    if items.iter().any(|i| i.side == "both") {
        warnings.push("Algunos mods también tienen que estar instalados en el Minecraft de cada jugador.".into());
    }
    let total_bytes = items.iter().filter_map(|i| i.version.primary_file()).map(|f| f.size).sum();
    Ok(InstallPlan { items, warnings, total_bytes })
}

fn loader_title(l: &str) -> &str { if l == "fabric" { "Fabric" } else { l } }

/// Ejecuta un plan: descarga cada archivo verificando su SHA-1 y actualiza el manifiesto.
pub async fn apply_plan(cfg: &ServerConfig, plan: &InstallPlan, mut progress: impl FnMut(&str, f64)) -> AppResult<Vec<InstalledMod>> {
    let dir = mods_dir(&cfg.id);
    std::fs::create_dir_all(&dir).map_err(|e| AppError::io("No se pudo crear la carpeta de mods", e))?;
    let mut manifest = load_manifest(&cfg.id);
    let mut done = vec![];
    let n = plan.items.len().max(1) as f64;
    for (i, item) in plan.items.iter().enumerate() {
        let file = item.version.primary_file().ok_or_else(|| AppError::invalid(format!("{} no tiene archivo descargable.", item.title)))?;
        if !file.url.starts_with("https://cdn.modrinth.com/") {
            return Err(AppError::invalid(format!("{} no viene del servidor oficial de Modrinth; se canceló por seguridad.", item.title)));
        }
        if file.filename.contains(['/', '\\']) || !file.filename.ends_with(".jar") {
            return Err(AppError::invalid(format!("Nombre de archivo sospechoso en {}; se canceló por seguridad.", item.title)));
        }
        progress(&format!("Descargando {}", item.title), i as f64 / n);
        let staging = paths::cache().join("mods").join(&file.hashes.sha1).with_extension("jar");
        if !staging.exists() || net::sha1_file(&staging).map(|h| h != file.hashes.sha1).unwrap_or(true) {
            net::download(&file.url, &staging, net::Hash::Sha1(&file.hashes.sha1), |_, _| {}).await?;
        }
        // Si reemplaza otra versión del mismo proyecto, se borra el jar viejo.
        if let Some(old) = manifest.mods.iter().find(|m| m.project_id == item.project_id) {
            let _ = std::fs::remove_file(dir.join(&old.file_name));
        }
        std::fs::copy(&staging, dir.join(&file.filename)).map_err(|e| AppError::io("No se pudo copiar el mod", e))?;
        let previous_explicit = manifest.mods.iter().find(|m| m.project_id == item.project_id).map(|m| m.explicit).unwrap_or(false);
        manifest.mods.retain(|m| m.project_id != item.project_id);
        let entry = InstalledMod {
            project_id: item.project_id.clone(),
            version_id: item.version.id.clone(),
            title: item.title.clone(),
            version_number: item.version.version_number.clone(),
            file_name: file.filename.clone(),
            sha1: file.hashes.sha1.clone(),
            side: item.side.clone(),
            explicit: item.explicit || previous_explicit,
            required_by: vec![],
            depends_on: item.depends_on.clone(),
        };
        manifest.mods.push(entry.clone());
        done.push(entry);
    }
    recompute_required_by(&mut manifest);
    save_manifest(&cfg.id, &manifest)?;
    progress("Listo", 1.0);
    Ok(done)
}

fn recompute_required_by(m: &mut Manifest) {
    let pairs: Vec<(String, String)> = m.mods.iter().flat_map(|x| x.depends_on.iter().map(move |d| (d.clone(), x.title.clone()))).collect();
    for x in &mut m.mods {
        x.required_by = pairs.iter().filter(|(d, _)| d == &x.project_id).map(|(_, t)| t.clone()).collect();
        x.required_by.sort();
        x.required_by.dedup();
    }
}

/// Quita un mod. Si otro lo necesita, se niega. Las dependencias que quedaron sin uso
/// (instaladas automáticamente) también se quitan.
pub fn remove(cfg: &ServerConfig, project_id: &str) -> AppResult<Vec<String>> {
    let mut m = load_manifest(&cfg.id);
    recompute_required_by(&mut m);
    let target = m.mods.iter().find(|x| x.project_id == project_id).ok_or_else(|| AppError::not_found("el mod"))?;
    if !target.required_by.is_empty() {
        return Err(AppError::invalid(format!("No se puede quitar «{}»: lo necesitan {}.", target.title, target.required_by.join(", "))));
    }
    let dir = mods_dir(&cfg.id);
    let mut removed = vec![];
    let mut to_remove: VecDeque<String> = VecDeque::from([project_id.to_string()]);
    while let Some(pid) = to_remove.pop_front() {
        let Some(pos) = m.mods.iter().position(|x| x.project_id == pid) else { continue };
        let gone = m.mods.remove(pos);
        let _ = std::fs::remove_file(dir.join(&gone.file_name));
        removed.push(gone.title.clone());
        recompute_required_by(&mut m);
        for dep in &gone.depends_on {
            if let Some(d) = m.mods.iter().find(|x| &x.project_id == dep) {
                if !d.explicit && d.required_by.is_empty() { to_remove.push_back(d.project_id.clone()); }
            }
        }
    }
    save_manifest(&cfg.id, &m)?;
    Ok(removed)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFileStatus {
    pub file_name: String,
    pub size_kb: u64,
    pub sha1: String,
    pub project_id: Option<String>,
    pub title: String,
    pub version_number: Option<String>,
    pub side: String,
    pub managed: bool,
    pub explicit: bool,
    pub required_by: Vec<String>,
    /// ok | duplicate | incompatible | client_only | unknown
    pub issues: Vec<String>,
    pub update: Option<String>,
}

/// Diagnóstico de la carpeta `mods`: identifica cada jar por hash en Modrinth y detecta
/// duplicados, versiones incompatibles y mods solo de cliente. No modifica nada.
/// Datos que trae el propio jar (sin internet): id, nombre, versión y en qué lado corre.
#[derive(Debug, Clone, PartialEq)]
pub struct JarMeta { pub id: String, pub name: String, pub version: String, pub environment: String, pub depends: Vec<String> }

/// Lee `fabric.mod.json` (Fabric) o `META-INF/neoforge.mods.toml` / `META-INF/mods.toml` (NeoForge/Forge).
pub fn jar_meta(path: &Path) -> Option<JarMeta> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut read = |name: &str| -> Option<String> {
        let mut e = z.by_name(name).ok()?;
        if e.size() > 1_000_000 { return None; }
        let mut s = String::new();
        std::io::Read::read_to_string(&mut e, &mut s).ok()?;
        Some(s)
    };
    if let Some(raw) = read("fabric.mod.json") {
        // Algunos mods traen saltos de línea sin escapar dentro de cadenas: se limpian.
        let clean: String = raw.chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect();
        let v: serde_json::Value = serde_json::from_str(&clean).ok()?;
        let get = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let env = get("environment");
        let depends = v.get("depends").and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default();
        return Some(JarMeta { id: get("id"), name: if get("name").is_empty() { get("id") } else { get("name") }, version: get("version"), environment: if env.is_empty() { "*".into() } else { env }, depends });
    }
    let toml = read("META-INF/neoforge.mods.toml").or_else(|| read("META-INF/mods.toml"))?;
    let field = |k: &str| toml.lines().map(str::trim).find(|l| l.starts_with(k) && l[k.len()..].trim_start().starts_with('='))
        .map(|l| l.splitn(2, '=').nth(1).unwrap_or("").trim().trim_matches('"').to_string()).unwrap_or_default();
    Some(JarMeta { id: field("modId"), name: field("displayName"), version: field("version"), environment: "*".into(), depends: vec![] })
}

pub async fn scan<S: Source>(src: &S, cfg: &ServerConfig, online: bool) -> AppResult<Vec<ModFileStatus>> {
    let loader = loader_id(cfg.loader).unwrap_or("fabric");
    let mc = cfg.mc_version.as_str();
    let dir = mods_dir(&cfg.id);
    let mut manifest = load_manifest(&cfg.id);
    recompute_required_by(&mut manifest);
    let by_file: HashMap<String, &InstalledMod> = manifest.mods.iter().map(|m| (m.file_name.clone(), m)).collect();

    let mut files: Vec<(String, u64, String)> = vec![];
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jar") { continue; }
            let size = e.metadata().map(|m| m.len() / 1024).unwrap_or(0);
            let sha = by_file.get(&name).map(|m| m.sha1.clone()).map(Ok).unwrap_or_else(|| net::sha1_file(&e.path()))?;
            files.push((name, size, sha));
        }
    }
    files.sort();

    let unknown_hashes: Vec<String> = files.iter().filter(|(n, _, _)| !by_file.contains_key(n)).map(|(_, _, h)| h.clone()).collect();
    let identified: HashMap<String, Version> = if online && !unknown_hashes.is_empty() { src.versions_by_sha1(&unknown_hashes).await.unwrap_or_default() } else { HashMap::new() };
    let mut project_ids: Vec<String> = identified.values().map(|v| v.project_id.clone()).collect();
    project_ids.extend(manifest.mods.iter().map(|m| m.project_id.clone()));
    project_ids.sort();
    project_ids.dedup();
    let projects: HashMap<String, Project> = if online { src.projects(&project_ids).await.unwrap_or_default().into_iter().map(|p| (p.id.clone(), p)).collect() } else { HashMap::new() };

    let mut out = vec![];
    for (name, size, sha) in files {
        let mut st = ModFileStatus { file_name: name.clone(), size_kb: size, sha1: sha.clone(), project_id: None, title: name.trim_end_matches(".jar").into(), version_number: None, side: "unknown".into(), managed: false, explicit: false, required_by: vec![], issues: vec![], update: None };
        if let Some(m) = by_file.get(&name) {
            st.project_id = Some(m.project_id.clone());
            st.title = m.title.clone();
            st.version_number = Some(m.version_number.clone());
            st.side = m.side.clone();
            st.managed = true;
            st.explicit = m.explicit;
            st.required_by = m.required_by.clone();
        } else if let Some(v) = identified.get(&sha) {
            st.project_id = Some(v.project_id.clone());
            st.version_number = Some(v.version_number.clone());
            st.title = v.name.clone();
            if !v.supports(mc, loader) { st.issues.push("incompatible".into()); }
        } else if online {
            st.issues.push("unknown".into());
        }
        if let Some(p) = st.project_id.as_ref().and_then(|id| projects.get(id)) {
            st.title = p.title.clone();
            st.side = modrinth::side_label(&p.client_side, &p.server_side).into();
            if !modrinth::runs_on_server(&p.server_side) { st.issues.push("client_only".into()); }
        }
        // Lo que dice el propio jar vale aunque no haya internet.
        let meta = jar_meta(&dir.join(&name));
        if let Some(m) = &meta {
            if !st.managed && st.project_id.is_none() && !m.name.is_empty() { st.title = m.name.clone(); }
            if st.version_number.is_none() && !m.version.is_empty() { st.version_number = Some(m.version.clone()); }
            if m.environment == "client" {
                st.side = "client".into();
                if !st.issues.iter().any(|i| i == "client_only") { st.issues.push("client_only".into()); }
            } else if m.environment == "server" && st.side == "unknown" {
                st.side = "server".into();
            }
            // Sin internet, "desconocido" solo si tampoco se pudo leer el jar.
            st.issues.retain(|i| i != "unknown" || m.id.is_empty());
        }
        out.push((st, meta.map(|m| m.id).unwrap_or_default()));
    }
    // Duplicados: dos archivos del mismo proyecto (o con el mismo id de mod).
    let key = |s: &ModFileStatus, mid: &str| s.project_id.clone().or_else(|| (!mid.is_empty()).then(|| format!("mod:{mid}")));
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for (s, mid) in &out { if let Some(k) = key(s, mid) { *count.entry(k).or_default() += 1; } }
    let mut result = vec![];
    for (mut s, mid) in out {
        if key(&s, &mid).map(|k| count[&k] > 1).unwrap_or(false) { s.issues.push("duplicate".into()); }
        result.push(s);
    }
    Ok(result)
}

/// Aparta un jar no administrado (por ejemplo, un duplicado) a la Papelera de 939 Host.
pub fn set_aside_file(cfg: &ServerConfig, file_name: &str) -> AppResult<()> {
    if file_name.contains(['/', '\\']) || !file_name.ends_with(".jar") { return Err(AppError::invalid("Nombre de archivo inválido.")); }
    let src = mods_dir(&cfg.id).join(file_name);
    if !src.exists() { return Err(AppError::not_found("el archivo")); }
    let dst_dir = paths::trash_slot(&format!("{}-mods", cfg.id));
    std::fs::create_dir_all(&dst_dir).map_err(|e| AppError::io("No se pudo crear la carpeta en la Papelera", e))?;
    std::fs::rename(&src, dst_dir.join(file_name)).map_err(|e| AppError::io("No se pudo apartar el archivo", e))?;
    let mut m = load_manifest(&cfg.id);
    m.mods.retain(|x| x.file_name != file_name);
    save_manifest(&cfg.id, &m)
}

/// Busca actualizaciones compatibles de los mods administrados.
pub async fn check_updates<S: Source>(src: &S, cfg: &ServerConfig) -> AppResult<HashMap<String, Version>> {
    let loader = loader_id(cfg.loader)?;
    let mut out = HashMap::new();
    for m in load_manifest(&cfg.id).mods {
        if let Ok(list) = src.project_versions(&m.project_id, &cfg.mc_version, loader).await {
            if let Some(best) = modrinth::best_version(list, &cfg.mc_version, loader) {
                if best.id != m.version_id { out.insert(m.project_id.clone(), best); }
            }
        }
    }
    Ok(out)
}

/// Mods de optimización de servidor recomendados para Fabric (verificados por versión al instalar).
pub const PERFORMANCE_FABRIC: &[(&str, &str, &str, bool)] = &[
    ("lithium", "Lithium", "Lógica de juego más eficiente.", true),
    ("ferrite-core", "FerriteCore", "Menos uso de memoria.", true),
    ("scalablelux", "ScalableLux", "Cálculo de luz mucho más rápido.", true),
    ("c2me-fabric", "C2ME", "Generación y carga de chunks en paralelo. Probalo con tus mods de mundo.", false),
    ("krypton", "Krypton", "Red más eficiente.", false),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::modrinth::{Dependency, Hashes, VersionFile};

    struct Fake { versions: Vec<Version>, projects: Vec<Project> }

    fn v(id: &str, project: &str, deps: Vec<(Option<&str>, Option<&str>, DependencyType)>, mc: &str, date: &str, kind: &str) -> Version {
        Version {
            id: id.into(), project_id: project.into(), name: format!("{project} {id}"), version_number: id.into(),
            game_versions: vec![mc.into()], loaders: vec!["fabric".into()], version_type: kind.into(), date_published: date.into(),
            files: vec![VersionFile { url: format!("https://cdn.example/{id}.jar"), filename: format!("{id}.jar"), primary: true, size: 1000, hashes: Hashes { sha1: format!("{id}sha"), sha512: String::new() } }],
            dependencies: deps.into_iter().map(|(p, vid, t)| Dependency { project_id: p.map(Into::into), version_id: vid.map(Into::into), file_name: None, dependency_type: t }).collect(),
        }
    }
    fn p(id: &str, client: &str, server: &str) -> Project {
        Project { id: id.into(), slug: id.into(), title: id.to_uppercase(), description: String::new(), client_side: client.into(), server_side: server.into(), icon_url: None }
    }
    impl Source for Fake {
        async fn version(&self, id: &str) -> AppResult<Version> { self.versions.iter().find(|x| x.id == id).cloned().ok_or_else(|| AppError::not_found(id)) }
        async fn project_versions(&self, project: &str, _mc: &str, _l: &str) -> AppResult<Vec<Version>> { Ok(self.versions.iter().filter(|x| x.project_id == project).cloned().collect()) }
        async fn projects(&self, ids: &[String]) -> AppResult<Vec<Project>> { Ok(self.projects.iter().filter(|x| ids.contains(&x.id)).cloned().collect()) }
        async fn versions_by_sha1(&self, hashes: &[String]) -> AppResult<HashMap<String, Version>> {
            Ok(self.versions.iter().filter_map(|x| { let h = x.files[0].hashes.sha1.clone(); hashes.contains(&h).then(|| (h, x.clone())) }).collect())
        }
    }

    fn fake() -> Fake {
        use DependencyType::*;
        Fake {
            versions: vec![
                v("a2", "a", vec![(Some("api"), None, Required), (Some("opt"), None, Optional), (Some("bad"), None, Incompatible)], "26.2", "2026-10-02", "release"),
                v("a1", "a", vec![], "26.2", "2026-09-01", "release"),
                v("a-old", "a", vec![], "26.1", "2026-11-01", "release"),
                v("api1", "api", vec![(None, Some("lib7"), Required)], "26.2", "2026-08-01", "release"),
                v("api2-beta", "api", vec![], "26.2", "2026-10-30", "beta"),
                v("lib7", "lib", vec![], "26.2", "2026-07-01", "release"),
                v("hud1", "hud", vec![], "26.2", "2026-07-01", "release"),
                v("bad1", "bad", vec![], "26.2", "2026-07-01", "release"),
            ],
            projects: vec![p("a", "required", "required"), p("api", "required", "required"), p("lib", "optional", "required"), p("hud", "required", "unsupported"), p("bad", "required", "required"), p("opt", "optional", "optional")],
        }
    }

    fn server(dir: &std::path::Path) -> ServerConfig {
        let _ = dir;
        let _ = paths::init(std::env::temp_dir().join("939host-unit-tests"));
        let id = format!("t{}", rand_id());
        std::fs::create_dir_all(paths::server_run_dir(&id)).unwrap();
        ServerConfig {
            id, name: "T".into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Fabric, loader_version: None, java_major: 25, java_required: 25, java_choice: None,
            memory_mb: 2048, jvm_profile: JvmProfile::Balanced, extra_jvm_args: String::new(), port: 25565, max_players: 8, motd: String::new(),
            online_mode: true, whitelist: true, difficulty: "hard".into(), gamemode: "survival".into(), hardcore: false, pvp: true, level_seed: String::new(),
            level_type: "minecraft:normal".into(), view_distance: 10, simulation_distance: 8, spawn_protection: 16, allow_flight: false, enable_command_block: false,
            sync_interval_min: 15, restore_interval_min: 15, installed: true, install_error: None, eula_accepted_at: None, created_at: String::new(),
            identity: Default::default(), extra_properties: Default::default(),
        }
    }
    fn rand_id() -> String { let mut b = [0u8; 4]; getrandom::getrandom(&mut b).unwrap(); hex::encode(b) }
    fn root() -> std::path::PathBuf { std::env::temp_dir().join("939host-mods-tests") }

    #[tokio::test]
    async fn plan_resolves_required_chain_and_picks_latest_release() {
        let cfg = server(&root());
        let plan = plan_install(&fake(), &cfg, "a", None).await.unwrap();
        let ids: Vec<&str> = plan.items.iter().map(|i| i.version.id.as_str()).collect();
        assert_eq!(ids, vec!["a2", "api1", "lib7"], "elige a2 (26.2 más nueva), api1 estable antes que la beta y lib7 por version_id");
        assert!(plan.items[0].explicit && !plan.items[1].explicit);
        assert!(!ids.contains(&"opt"), "las opcionales no se instalan solas");
        assert_eq!(plan.total_bytes, 3000);
    }

    #[tokio::test]
    async fn plan_rejects_incompatible_with_installed() {
        let cfg = server(&root());
        save_manifest(&cfg.id, &Manifest { mods: vec![InstalledMod { project_id: "bad".into(), version_id: "bad1".into(), title: "BAD".into(), version_number: "1".into(), file_name: "bad1.jar".into(), sha1: "x".into(), side: "both".into(), explicit: true, required_by: vec![], depends_on: vec![] }] }).unwrap();
        let err = plan_install(&fake(), &cfg, "a", None).await.unwrap_err();
        assert!(err.message.contains("incompatible"), "{}", err.message);
    }

    #[tokio::test]
    async fn plan_rejects_client_only_mod() {
        let cfg = server(&root());
        let err = plan_install(&fake(), &cfg, "hud", None).await.unwrap_err();
        assert!(err.message.contains("solo de cliente"));
    }

    #[tokio::test]
    async fn plan_rejects_wrong_game_version_and_vanilla() {
        let mut cfg = server(&root());
        assert!(plan_install(&fake(), &cfg, "a", Some("a-old")).await.is_err());
        cfg.loader = Loader::Vanilla;
        assert!(plan_install(&fake(), &cfg, "a", None).await.unwrap_err().message.contains("Vanilla"));
    }

    #[tokio::test]
    async fn remove_protects_dependencies_and_cleans_orphans() {
        let cfg = server(&root());
        let plan = plan_install(&fake(), &cfg, "a", None).await.unwrap();
        // Simula la instalación sin red: escribe el manifiesto y archivos vacíos.
        let dir = mods_dir(&cfg.id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest::default();
        for it in &plan.items {
            let f = it.version.primary_file().unwrap();
            std::fs::write(dir.join(&f.filename), b"jar").unwrap();
            m.mods.push(InstalledMod { project_id: it.project_id.clone(), version_id: it.version.id.clone(), title: it.title.clone(), version_number: it.version.version_number.clone(), file_name: f.filename.clone(), sha1: f.hashes.sha1.clone(), side: it.side.clone(), explicit: it.explicit, required_by: vec![], depends_on: it.depends_on.clone() });
        }
        save_manifest(&cfg.id, &m).unwrap();
        let err = remove(&cfg, "api").unwrap_err();
        assert!(err.message.contains("lo necesitan"), "{}", err.message);
        let removed = remove(&cfg, "a").unwrap();
        assert_eq!(removed.len(), 3, "quita a, api y lib (dependencias huérfanas): {removed:?}");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        assert!(load_manifest(&cfg.id).mods.is_empty());
    }

    #[tokio::test]
    async fn scan_detects_duplicates_client_only_and_unknown() {
        let cfg = server(&root());
        let dir = mods_dir(&cfg.id);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["one.jar", "two.jar", "hud.jar", "mystery.jar"] { std::fs::write(dir.join(name), name.as_bytes()).unwrap(); }
        let sha = |n: &str| net::sha1_file(&dir.join(n)).unwrap();
        let mut f = fake();
        // Dos archivos distintos del mismo proyecto "a", uno solo-cliente, uno desconocido.
        f.versions[0].files[0].hashes.sha1 = sha("one.jar");
        f.versions[1].files[0].hashes.sha1 = sha("two.jar");
        f.versions.iter_mut().find(|x| x.id == "hud1").unwrap().files[0].hashes.sha1 = sha("hud.jar");
        let st = scan(&f, &cfg, true).await.unwrap();
        let get = |n: &str| st.iter().find(|s| s.file_name == n).unwrap();
        assert!(get("one.jar").issues.contains(&"duplicate".to_string()));
        assert!(get("two.jar").issues.contains(&"duplicate".to_string()));
        assert!(get("hud.jar").issues.contains(&"client_only".to_string()));
        assert!(get("mystery.jar").issues.contains(&"unknown".to_string()));
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub copied: Vec<String>,
    pub skipped_client: Vec<String>,
    pub skipped_duplicate: Vec<String>,
    pub skipped_existing: Vec<String>,
    pub not_mods: Vec<String>,
}

/// Copia los mods de otra carpeta (por ejemplo, la de un perfil de Minecraft) al servidor.
/// Solo lee la carpeta de origen. Omite los mods solo de cliente y los duplicados (se queda con
/// el archivo más nuevo de cada mod) y nunca pisa archivos que el servidor ya tenga.
pub fn import_folder(cfg: &ServerConfig, src: &Path) -> AppResult<ImportReport> {
    if !src.is_dir() { return Err(AppError::invalid("La carpeta no existe.")); }
    let files: Vec<PathBuf> = std::fs::read_dir(src).map_err(|e| AppError::io("No se pudo leer la carpeta", e))?.flatten().map(|e| e.path()).collect();
    import_files(cfg, &files)
}

/// Importa archivos sueltos (arrastrados o elegidos): mismas reglas que una carpeta.
pub fn import_files(cfg: &ServerConfig, files: &[PathBuf]) -> AppResult<ImportReport> {
    let dest = mods_dir(&cfg.id);
    std::fs::create_dir_all(&dest).map_err(|e| AppError::io("No se pudo crear la carpeta de mods", e))?;
    let mut present: HashSet<String> = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(&dest) {
        for e in rd.flatten() { if let Some(m) = jar_meta(&e.path()) { present.insert(m.id); } }
    }
    let mut best: HashMap<String, (std::time::SystemTime, PathBuf, String)> = HashMap::new();
    let mut r = ImportReport { copied: vec![], skipped_client: vec![], skipped_duplicate: vec![], skipped_existing: vec![], not_mods: vec![] };
    let mut metas = vec![];
    for p in files {
        if !p.is_file() { continue; }
        let name = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if !name.to_lowercase().ends_with(".jar") { if !name.is_empty() { r.not_mods.push(name); } continue; }
        let mtime = std::fs::metadata(p).and_then(|x| x.modified()).unwrap_or(std::time::UNIX_EPOCH);
        match jar_meta(p) { Some(m) => metas.push((mtime, p.clone(), name, m)), None => r.not_mods.push(name) }
    }
    // Un mod "solo de cliente" se copia igual si otro mod del servidor lo declara como dependencia:
    // Fabric no lo carga en el servidor, pero exige que esté presente.
    let needed: HashSet<String> = metas.iter().filter(|(_, _, _, m)| m.environment != "client").flat_map(|(_, _, _, m)| m.depends.clone()).collect();
    for (mtime, p, name, m) in metas {
        if m.environment == "client" && !needed.contains(&m.id) { r.skipped_client.push(format!("{} ({name})", if m.name.is_empty() { &m.id } else { &m.name })); continue; }
        if present.contains(&m.id) { r.skipped_existing.push(name); continue; }
        match best.get(&m.id) {
            Some((t, _, _)) if *t >= mtime => r.skipped_duplicate.push(name),
            Some((_, _, old)) => { r.skipped_duplicate.push(old.clone()); best.insert(m.id.clone(), (mtime, p, name)); }
            None => { best.insert(m.id.clone(), (mtime, p, name)); }
        }
    }
    for (_, (_, p, name)) in best {
        let target = dest.join(&name);
        if target.exists() { r.skipped_existing.push(name); continue; }
        std::fs::copy(&p, &target).map_err(|e| AppError::io("No se pudo copiar un mod", e))?;
        r.copied.push(name);
    }
    r.copied.sort();
    Ok(r)
}

/// Vincula un archivo agregado a mano con su proyecto de Modrinth (identificado por hash), para
/// que 939 Host lo administre: actualizaciones, dependencias y quitar con un clic. No descarga
/// ni cambia el archivo.
pub async fn link_file<S: Source>(src: &S, cfg: &ServerConfig, file_name: &str) -> AppResult<InstalledMod> {
    if file_name.contains(['/', '\\']) || !file_name.ends_with(".jar") { return Err(AppError::invalid("Archivo inválido.")); }
    let path = mods_dir(&cfg.id).join(file_name);
    if !path.is_file() { return Err(AppError::not_found("el archivo")); }
    let mut manifest = load_manifest(&cfg.id);
    if manifest.mods.iter().any(|m| m.file_name == file_name) { return Err(AppError::invalid("Ese archivo ya está administrado por 939 Host.")); }
    let sha = net::sha1_file(&path)?;
    let found = src.versions_by_sha1(std::slice::from_ref(&sha)).await?;
    let v = found.get(&sha).ok_or_else(|| AppError::invalid("Modrinth no reconoce este archivo: no se puede vincular."))?;
    if manifest.mods.iter().any(|m| m.project_id == v.project_id) { return Err(AppError::invalid("Ese mod ya está instalado con otro archivo. Apartá el duplicado.")); }
    let project = src.projects(std::slice::from_ref(&v.project_id)).await?.into_iter().next();
    let m = InstalledMod {
        project_id: v.project_id.clone(),
        version_id: v.id.clone(),
        title: project.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| v.name.clone()),
        version_number: v.version_number.clone(),
        file_name: file_name.to_string(),
        sha1: sha,
        side: project.as_ref().map(|p| modrinth::side_label(&p.client_side, &p.server_side).to_string()).unwrap_or_else(|| "unknown".into()),
        explicit: true,
        required_by: vec![],
        depends_on: vec![],
    };
    manifest.mods.push(m.clone());
    save_manifest(&cfg.id, &manifest)?;
    Ok(m)
}
