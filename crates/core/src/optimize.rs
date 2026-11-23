//! Mods de rendimiento y optimización, elegidos para cada versión y loader.
//!
//! La lista de candidatos es conocimiento curado (qué hace cada mod, si es estable o
//! experimental, con qué choca); **la disponibilidad se decide en el momento** consultando a
//! Modrinth: si un mod no tiene versión para esa combinación exacta de Minecraft y loader, no se
//! ofrece. Los que solo funcionan en el cliente se descartan. Las dependencias y las
//! incompatibilidades declaradas en Modrinth las resuelve el gestor de mods de siempre
//! (`mods::plan_install`), y las descargas se verifican por hash.
//!
//! Criterio de selección (documentación y páginas oficiales de cada proyecto):
//! - Lithium: optimiza lógica del juego (IA, física, ticks de chunks) sin cambiar el
//!   comportamiento vanilla. Fabric y NeoForge.
//! - FerriteCore: reduce la memoria usada por estados de bloques y modelos. Fabric/Forge/NeoForge.
//! - ModernFix: acorta el arranque y reduce memoria. Fabric/Forge/NeoForge.
//! - Krypton: optimiza la capa de red del servidor. Fabric.
//! - Canary: adaptación de Lithium para Forge (versiones donde Lithium no existe para Forge).
//! - C2ME: genera y carga chunks en paralelo. Muy efectivo, pero más agresivo: «Máximo».
//! - Noisium: acelera la generación de terreno (en versiones nuevas, parte ya está en el juego).
//! - ServerCore: optimizaciones y ajustes para servidores; algunas opciones cambian el
//!   comportamiento (por ejemplo, la actividad de mobs lejanos): «Máximo».
//! - Alternate Current: redstone mucho más rápida, pero cambia el orden de actualización y puede
//!   romper máquinas que dependen del comportamiento vanilla: solo personalizado, experimental.
//! - Chunky: herramienta para pregenerar el mundo. No hace nada hasta que se lo pide el usuario.

use crate::error::{AppError, AppResult};
use crate::model::{Loader, ServerConfig};
use crate::modrinth::{DependencyType, Source};
use crate::mods;
use serde::Serialize;

pub struct Known {
    pub slug: &'static str,
    pub what: &'static str,
    /// recommended | max | custom
    pub tier: &'static str,
    pub experimental: bool,
    pub loaders: &'static [&'static str],
    /// Otros candidatos con los que no conviene combinarlo (además de lo que declare Modrinth).
    pub avoid_with: &'static [&'static str],
}

pub const CATALOG: &[Known] = &[
    Known { slug: "lithium", what: "Lógica del juego más eficiente: mobs, física y ticks de chunks, sin cambiar cómo se juega.", tier: "recommended", experimental: false, loaders: &["fabric", "neoforge"], avoid_with: &["canary"] },
    Known { slug: "ferrite-core", what: "Menos memoria RAM ocupada por bloques y modelos.", tier: "recommended", experimental: false, loaders: &["fabric", "forge", "neoforge"], avoid_with: &[] },
    Known { slug: "modernfix", what: "Arranque más rápido y menos memoria.", tier: "recommended", experimental: false, loaders: &["fabric", "forge", "neoforge"], avoid_with: &[] },
    Known { slug: "krypton", what: "Red del servidor más liviana: menos CPU por cada jugador conectado.", tier: "recommended", experimental: false, loaders: &["fabric"], avoid_with: &[] },
    Known { slug: "canary", what: "Versión de Lithium para Forge: lógica del juego más eficiente.", tier: "recommended", experimental: false, loaders: &["forge"], avoid_with: &["lithium"] },
    Known { slug: "chunky", what: "Pregenera el mundo para que explorar no trabe al servidor. Solo trabaja cuando vos lo iniciás.", tier: "recommended", experimental: false, loaders: &["fabric", "forge", "neoforge"], avoid_with: &[] },
    Known { slug: "c2me-fabric", what: "Genera y carga chunks usando varios núcleos del procesador.", tier: "max", experimental: false, loaders: &["fabric"], avoid_with: &["noisium"] },
    Known { slug: "noisium", what: "Generación de terreno más rápida.", tier: "max", experimental: false, loaders: &["fabric", "neoforge"], avoid_with: &["c2me-fabric"] },
    Known { slug: "servercore", what: "Optimizaciones para servidores; algunas opciones reducen la actividad de mobs lejanos.", tier: "max", experimental: false, loaders: &["fabric", "neoforge"], avoid_with: &[] },
    Known { slug: "alternate-current", what: "Redstone mucho más rápida. Cambia el orden de actualización: puede romper máquinas de redstone.", tier: "custom", experimental: true, loaders: &["fabric"], avoid_with: &[] },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub slug: String,
    pub project_id: Option<String>,
    pub title: String,
    pub icon_url: Option<String>,
    pub summary: String,
    pub what: String,
    pub tier: String,
    pub experimental: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub version_number: Option<String>,
    pub version_id: Option<String>,
    /// required | optional | unsupported (en los jugadores)
    pub client: String,
    pub dependencies: Vec<String>,
    pub avoid_with: Vec<String>,
    pub installed: bool,
    /// "catalog": elegido y revisado por 939 Host. "modrinth": encontrado ahora en la categoría
    /// «optimización» de Modrinth para esta versión exacta (nunca entra solo en un preset).
    pub source: String,
}

/// Qué candidatos sirven para este Minecraft y loader, consultando a Modrinth.
pub async fn candidates<S: Source>(src: &S, mc: &str, loader: Loader, installed: &[String]) -> AppResult<Vec<Candidate>> {
    let lid = mods::loader_id(loader)?;
    let list: Vec<&Known> = CATALOG.iter().filter(|k| k.loaders.contains(&lid)).collect();
    let slugs: Vec<String> = list.iter().map(|k| k.slug.to_string()).collect();
    let projects = src.projects(&slugs).await.unwrap_or_default();
    let mut out = vec![];
    for k in list {
        let p = projects.iter().find(|p| p.slug == k.slug);
        let mut c = Candidate {
            slug: k.slug.into(), project_id: p.map(|p| p.id.clone()), title: p.map(|p| p.title.clone()).unwrap_or_else(|| k.slug.into()),
            icon_url: p.and_then(|p| p.icon_url.clone()), summary: p.map(|p| p.description.clone()).unwrap_or_default(), what: k.what.into(),
            tier: k.tier.into(), experimental: k.experimental, available: false, unavailable_reason: None, version_number: None, version_id: None,
            client: p.map(|p| p.client_side.clone()).unwrap_or_else(|| "unknown".into()), dependencies: vec![],
            avoid_with: k.avoid_with.iter().map(|s| s.to_string()).collect(), installed: false, source: "catalog".into(),
        };
        let Some(p) = p else { c.unavailable_reason = Some("No se encontró en Modrinth.".into()); out.push(c); continue };
        c.installed = installed.contains(&p.id);
        if p.server_side == "unsupported" { c.unavailable_reason = Some("Solo funciona en el Minecraft de cada jugador, no en el servidor.".into()); out.push(c); continue; }
        match mods::best_compatible(src, &p.id, mc, lid).await {
            Some(v) => {
                c.available = true;
                c.version_number = Some(v.version_number.clone());
                c.version_id = Some(v.id.clone());
                c.dependencies = v.dependencies.iter().filter(|d| d.dependency_type == DependencyType::Required).filter_map(|d| d.project_id.clone()).collect();
            }
            None => c.unavailable_reason = Some(format!("No tiene versión para {} {mc}.", loader.name())),
        }
        out.push(c);
    }
    // Además del catálogo: lo que Modrinth tiene publicado en «optimización» para esta versión
    // exacta y que corre en el servidor (así una versión vieja, como 1.14, no queda en «ninguno»
    // si existe algo). Se comprueba que tenga un archivo para la versión y el loader.
    let hits = src.search_optimization(mc, lid).await.unwrap_or_default();
    for h in hits.into_iter().filter(|h| !CATALOG.iter().any(|k| k.slug == h.slug)
        && (h.server_side == "required" || h.server_side == "optional")).take(14) {
        let Some(v) = mods::best_compatible(src, &h.project_id, mc, lid).await else { continue };
        out.push(Candidate {
            slug: h.slug.clone(), project_id: Some(h.project_id.clone()), title: h.title.clone(), icon_url: h.icon_url.clone(), summary: h.description.clone(),
            what: h.description.clone(), tier: "custom".into(), experimental: false, available: true, unavailable_reason: None,
            version_number: Some(v.version_number.clone()), version_id: Some(v.id.clone()), client: h.client_side.clone(),
            dependencies: v.dependencies.iter().filter(|d| d.dependency_type == DependencyType::Required).filter_map(|d| d.project_id.clone()).collect(),
            avoid_with: vec![], installed: installed.contains(&h.project_id), source: "modrinth".into(),
        });
    }
    // Los nombres de las dependencias, para mostrarlas.
    let dep_ids: Vec<String> = out.iter().flat_map(|c| c.dependencies.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    if !dep_ids.is_empty() {
        if let Ok(dp) = src.projects(&dep_ids).await {
            for c in out.iter_mut() { for d in c.dependencies.iter_mut() { if let Some(p) = dp.iter().find(|p| &p.id == d) { *d = p.title.clone(); } } }
        }
    }
    Ok(out)
}

/// Selección de un preset entre los candidatos disponibles, sin combinaciones a evitar
/// (gana el que aparece primero en el catálogo, que es el más conservador).
pub fn preset(cands: &[Candidate], which: &str) -> Vec<String> {
    let tiers: &[&str] = match which { "max" => &["recommended", "max"], _ => &["recommended"] };
    let mut pick: Vec<String> = vec![];
    for c in cands.iter().filter(|c| c.available && !c.experimental && c.source == "catalog" && tiers.contains(&c.tier.as_str())) {
        if c.avoid_with.iter().any(|a| pick.contains(a)) { continue; }
        pick.push(c.slug.clone());
    }
    pick
}

/// Pares elegidos que no conviene combinar (para avisar en «Personalizado»).
pub fn conflicts(cands: &[Candidate], chosen: &[String]) -> Vec<(String, String)> {
    let mut v = vec![];
    for c in cands.iter().filter(|c| chosen.contains(&c.slug)) {
        for a in &c.avoid_with { if chosen.contains(a) && c.slug < *a { v.push((c.slug.clone(), a.clone())); } }
    }
    v
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub installed: Vec<String>,
    pub already: Vec<String>,
    pub skipped: Vec<(String, String)>,
    pub client_needed: Vec<String>,
    pub warnings: Vec<String>,
}

/// Instala los elegidos con el gestor de mods (dependencias, incompatibilidades y hashes).
/// Se marcan como instalados desde «Optimización» para mostrarlos así en la lista de mods.
pub async fn install<S: Source>(src: &S, cfg: &ServerConfig, slugs: &[String], mut progress: impl FnMut(&str, f64)) -> AppResult<InstallReport> {
    let lid = mods::loader_id(cfg.loader)?;
    let mut rep = InstallReport::default();
    let cands = candidates(src, &cfg.mc_version, cfg.loader, &mods::load_manifest(&cfg.id).mods.iter().map(|m| m.project_id.clone()).collect::<Vec<_>>()).await?;
    let bad = conflicts(&cands, slugs);
    if let Some((a, b)) = bad.first() { return Err(AppError::invalid(format!("{a} y {b} no conviene instalarlos juntos: elegí uno."))); }
    let n = slugs.len().max(1) as f64;
    for (i, slug) in slugs.iter().enumerate() {
        let Some(c) = cands.iter().find(|c| &c.slug == slug) else { rep.skipped.push((slug.clone(), "no está en la lista de optimización".into())); continue };
        if !c.available { rep.skipped.push((c.title.clone(), c.unavailable_reason.clone().unwrap_or_default())); continue; }
        if c.installed { rep.already.push(c.title.clone()); continue; }
        progress(&format!("Instalando {}", c.title), i as f64 / n);
        let pid = c.project_id.clone().unwrap();
        match mods::plan_install(src, cfg, &pid, c.version_id.as_deref()).await {
            Ok(plan) => {
                rep.warnings.extend(plan.warnings.clone());
                if let Err(e) = mods::apply_plan(cfg, &plan, |_, _| {}).await { rep.skipped.push((c.title.clone(), e.message)); continue; }
                let mut m = mods::load_manifest(&cfg.id);
                if let Some(x) = m.mods.iter_mut().find(|x| x.project_id == pid) { x.origin = Some("optimizacion".into()); }
                mods::save_manifest(&cfg.id, &m)?;
                rep.installed.push(c.title.clone());
                if c.client == "required" { rep.client_needed.push(c.title.clone()); }
            }
            Err(e) => rep.skipped.push((c.title.clone(), e.message)),
        }
    }
    let _ = lid;
    rep.warnings.sort(); rep.warnings.dedup();
    progress("Listo", 1.0);
    Ok(rep)
}

/* Pendientes de instalar al crear el servidor (se instalan cuando termina la instalación). */
fn pending_path(id: &str) -> std::path::PathBuf { crate::paths::server_dir(id).join("optimizacion-pendiente.json") }
pub fn set_pending(id: &str, slugs: &[String]) -> AppResult<()> {
    if slugs.is_empty() { let _ = std::fs::remove_file(pending_path(id)); return Ok(()); }
    crate::paths::write_json(&pending_path(id), &slugs)
}
pub fn take_pending(id: &str) -> Vec<String> {
    let v: Vec<String> = crate::paths::read_json(&pending_path(id)).unwrap_or_default();
    let _ = std::fs::remove_file(pending_path(id));
    v
}

#[cfg(test)]
mod t {
    use super::*;
    fn c(slug: &str, tier: &str, avail: bool, avoid: &[&str]) -> Candidate {
        Candidate { slug: slug.into(), project_id: Some(slug.into()), title: slug.into(), icon_url: None, summary: String::new(), what: String::new(), tier: tier.into(),
            experimental: tier == "custom", available: avail, unavailable_reason: None, version_number: None, version_id: None, client: "optional".into(),
            dependencies: vec![], avoid_with: avoid.iter().map(|s| s.to_string()).collect(), installed: false, source: "catalog".into() }
    }
    #[test]
    fn presets_respect_tiers_availability_and_conflicts() {
        let cs = vec![c("lithium", "recommended", true, &[]), c("krypton", "recommended", false, &[]), c("chunky", "recommended", true, &[]),
            c("c2me-fabric", "max", true, &["noisium"]), c("noisium", "max", true, &["c2me-fabric"]), c("alternate-current", "custom", true, &[])];
        assert_eq!(preset(&cs, "recommended"), vec!["lithium", "chunky"]);
        assert_eq!(preset(&cs, "max"), vec!["lithium", "chunky", "c2me-fabric"]);
        assert_eq!(conflicts(&cs, &["c2me-fabric".into(), "noisium".into()]).len(), 1);
        assert!(conflicts(&cs, &["lithium".into(), "chunky".into()]).is_empty());
    }
    #[test]
    fn catalog_is_consistent() {
        for k in CATALOG {
            for a in k.avoid_with { assert!(CATALOG.iter().any(|x| x.slug == *a), "{a}"); }
            assert!(["recommended", "max", "custom"].contains(&k.tier));
        }
    }
}
