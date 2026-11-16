//! Cambiar la versión de Minecraft de un servidor.
//!
//! Qué implica de verdad (documentación de Minecraft y de cada loader):
//! - **Actualizar** (versión más nueva): el servidor convierte el mundo al abrirlo (DataFixer).
//!   Después de convertido, ese mundo no se puede volver a abrir con la versión anterior sin
//!   riesgo de perder datos. Por eso antes se hace una copia completa y verificada.
//! - **Volver a una versión anterior**: Minecraft no convierte mundos hacia atrás. Abrir un
//!   mundo más nuevo con una versión vieja puede borrar bloques, ítems o chunks. No se hace sobre
//!   el mismo servidor: solo como copia (sin el mundo o asumiendo el riesgo) o servidor nuevo.
//! - **Snapshots**: un mundo abierto en una snapshot puede no volver a abrirse en una versión
//!   estable.
//! - **Loader**: Fabric necesita su loader para la versión nueva; Forge y NeoForge se reinstalan
//!   con su instalador oficial para esa versión (las librerías viejas no sirven).
//! - **Mods**: cada mod tiene versiones por versión de Minecraft. Los que vienen de Modrinth se
//!   buscan para la versión destino; los agregados a mano no se pueden verificar.
//! - **Java**: cada versión pide un Java mínimo (lo dice Mojang); se instala el que haga falta.

use crate::error::{AppError, AppResult};
use crate::model::{Loader, RunStatus, ServerConfig};
use crate::modrinth::Source;
use crate::process::Manager;
use crate::{backup, instance, java, meta, mods, paths};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModCheck {
    pub file: String,
    pub title: String,
    pub project_id: Option<String>,
    /// ok | update | missing | unknown
    pub status: String,
    pub current: String,
    pub target: Option<String>,
    pub target_version_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub from_mc: String,
    pub to_mc: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub loader_supported: bool,
    /// upgrade | downgrade | same
    pub direction: String,
    pub snapshot_involved: bool,
    pub java_from: u32,
    pub java_to: u32,
    pub has_world: bool,
    /// Actualizar en el mismo servidor es posible y seguro (con copia previa).
    pub can_update_in_place: bool,
    /// Se puede volver atrás restaurando la copia que se hace antes.
    pub rollback_by_backup: bool,
    pub mods: Vec<ModCheck>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub keeps: Vec<String>,
}

fn loader_id(l: Loader) -> &'static str { mods::loader_id(l).unwrap_or("") }

/// Analiza qué implica pasar `cfg` a `to_mc` (sin tocar nada).
pub async fn analyze<S: Source>(src: &S, cfg: &ServerConfig, to_mc: &str) -> AppResult<Analysis> {
    let mut blockers = vec![];
    let mut warnings = vec![];
    let from = meta::version_info(&cfg.mc_version).await?;
    let to = meta::version_info(to_mc).await?.ok_or_else(|| AppError::not_found(&format!("Minecraft {to_mc}")))?;
    let direction = match &from {
        _ if cfg.mc_version == to_mc => "same",
        Some((ft, _)) if to.0 > *ft => "upgrade",
        Some(_) => "downgrade",
        None => "upgrade",
    }.to_string();
    let snapshot_involved = to.1 != "release" || from.as_ref().map(|f| f.1 != "release").unwrap_or(false);
    let java_to = match meta::server_download(to_mc).await {
        Ok(d) => java::install_major(d.java_major),
        Err(_) => { blockers.push(format!("Minecraft {to_mc} no tiene servidor oficial de Mojang.")); 0 }
    };
    let (loader_supported, loader_version) = if cfg.loader == Loader::Vanilla { (true, None) } else {
        match meta::loader_support(cfg.loader, to_mc).await {
            Ok(s) if s.supported => (true, s.loader_version),
            Ok(_) => { blockers.push(format!("{} todavía no tiene versión para Minecraft {to_mc}.", cfg.loader.name())); (false, None) }
            Err(e) => { blockers.push(format!("No se pudo consultar {}: {}", cfg.loader.name(), e.message)); (false, None) }
        }
    };
    let world = paths::server_run_dir(&cfg.id).join("world");
    let has_world = world.join("level.dat").exists();
    if direction == "same" { blockers.push(format!("El servidor ya usa Minecraft {to_mc}.")); }
    if direction == "downgrade" && has_world {
        blockers.push(format!("Minecraft no convierte mundos hacia atrás: abrir este mundo con {to_mc} puede borrar bloques, ítems o zonas. Hacé una copia sin el mundo o un servidor nuevo."));
    }
    if direction == "upgrade" && has_world {
        warnings.push(format!("Minecraft {to_mc} actualiza el mundo al abrirlo. Después ya no se puede abrir con {} sin riesgo de perder datos: antes se crea una copia completa y verificada para poder volver.", cfg.mc_version));
    }
    if snapshot_involved {
        warnings.push("Las snapshots son versiones de prueba: un mundo abierto en una snapshot puede no volver a abrirse en una versión estable.".into());
    }

    // Mods: los de Modrinth se buscan para la versión nueva; los agregados a mano no se pueden verificar.
    let mut checks = vec![];
    if cfg.loader != Loader::Vanilla {
        let manifest = mods::load_manifest(&cfg.id);
        let dir = mods::mods_dir(&cfg.id);
        let mut known: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in &manifest.mods {
            if !dir.join(&m.file_name).exists() { continue; }
            known.insert(m.file_name.clone());
            let best = src.project_versions(&m.project_id, to_mc, loader_id(cfg.loader)).await.ok()
                .and_then(|v| crate::modrinth::best_version(v, to_mc, loader_id(cfg.loader)));
            let (status, target, tid) = match best {
                Some(v) if v.id == m.version_id => ("ok", Some(v.version_number.clone()), Some(v.id.clone())),
                Some(v) => ("update", Some(v.version_number.clone()), Some(v.id.clone())),
                None => ("missing", None, None),
            };
            checks.push(ModCheck { file: m.file_name.clone(), title: m.title.clone(), project_id: Some(m.project_id.clone()), status: status.into(), current: m.version_number.clone(), target, target_version_id: tid });
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".jar") || known.contains(&name) { continue; }
            let jm = mods::jar_meta(&e.path());
            checks.push(ModCheck { title: jm.as_ref().map(|j| j.name.clone()).unwrap_or_else(|| name.clone()), current: jm.map(|j| j.version).unwrap_or_default(), file: name, project_id: None, status: "unknown".into(), target: None, target_version_id: None });
        }
        let missing: Vec<&str> = checks.iter().filter(|c| c.status == "missing").map(|c| c.title.as_str()).collect();
        if !missing.is_empty() {
            warnings.push(format!("{} {} no {} versión para Minecraft {to_mc}: {}. Se apartan (no se borran) y el servidor arranca sin ellos.",
                missing.len(), if missing.len() == 1 { "mod" } else { "mods" }, if missing.len() == 1 { "tiene" } else { "tienen" }, missing.join(", ")));
        }
        let unknown = checks.iter().filter(|c| c.status == "unknown").count();
        if unknown > 0 {
            warnings.push(format!("{unknown} {} agregado a mano: 939 Host no puede saber si sirve para {to_mc}. Se deja como está; si el servidor no arranca, el asistente de reparación lo identifica.", if unknown == 1 { "mod fue" } else { "mods fueron" }));
        }
        let upd = checks.iter().filter(|c| c.status == "update").count();
        if upd > 0 { warnings.push(format!("{upd} {} a la versión para {to_mc} (descarga verificada desde Modrinth).", if upd == 1 { "mod se actualiza" } else { "mods se actualizan" })); }
    }
    if java_to != 0 && java_to != cfg.java_major { warnings.push(format!("Usa Java {java_to} (ahora Java {}). 939 Host lo descarga y lo verifica.", cfg.java_major)); }
    let mut keeps = vec!["La configuración del servidor, jugadores, lista blanca y operadores".to_string(), "Las copias y puntos de restauración anteriores".to_string(), "La identidad del servidor y su grupo".to_string()];
    if has_world { keeps.insert(0, "El mundo (convertido a la versión nueva)".into()); }
    let can = blockers.is_empty() && direction == "upgrade";
    Ok(Analysis {
        from_mc: cfg.mc_version.clone(), to_mc: to_mc.into(), loader: cfg.loader.name().into(), loader_version, loader_supported, direction, snapshot_involved,
        java_from: cfg.java_major, java_to, has_world, can_update_in_place: can, rollback_by_backup: can, mods: checks, blockers, warnings, keeps,
    })
}

/// Registro para poder volver al estado anterior, incluso después de reiniciar la app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRecord {
    pub from_mc: String,
    pub from_loader_version: Option<String>,
    pub from_java_choice: Option<u32>,
    pub to_mc: String,
    pub backup_file: String,
    /// Carpeta de la Papelera con los mods que se reemplazaron o apartaron.
    pub mods_aside: Option<String>,
    pub at: String,
    pub verified: bool,
}

fn record_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("cambio-de-version.json") }
pub fn last_change(id: &str) -> Option<ChangeRecord> { paths::read_json(&record_path(id)).ok() }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeReport {
    pub ok: bool,
    pub backup_file: String,
    pub steps: Vec<String>,
    pub error: Option<String>,
    pub can_rollback: bool,
}

/// Archivos del servidor que dependen de la versión (se reemplazan). El mundo, `config/`,
/// `mods/`, propiedades y listas de jugadores no se tocan.
fn clear_version_files(run: &std::path::Path) {
    for f in ["server.jar", "fabric-server-launch.jar", "fabric-server-launcher.properties", "run.bat", "run.sh"] { let _ = std::fs::remove_file(run.join(f)); }
    for d in ["libraries", "versions", ".fabric"] { let _ = std::fs::remove_dir_all(run.join(d)); }
}

/// Actualiza el servidor en el mismo lugar. Pasos: copia verificada → mods → componentes →
/// arranque de prueba con verificación real de conexión → apagado. Si algo falla después de la
/// copia, el informe ofrece volver atrás (`rollback`).
pub async fn update_in_place<S: Source>(mgr: &Manager, src: &S, id: &str, to_mc: &str, mut step: impl FnMut(&str, f64)) -> AppResult<ChangeReport> {
    let cfg = instance::load(id)?;
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de cambiar la versión.")); }
    step("Analizando el servidor", 0.02);
    let a = analyze(src, &cfg, to_mc).await?;
    if !a.can_update_in_place { return Err(AppError::invalid(a.blockers.first().cloned().unwrap_or_else(|| "Este cambio no se puede hacer sobre el mismo servidor.".into()))); }
    let mut steps = vec![];

    // 1. Copia completa y verificada.
    step("Creando una copia de seguridad completa", 0.08);
    let b = backup::create(mgr, id, &format!("Antes de pasar a {to_mc}"), "seguridad", &cfg.mc_version)?;
    let zip_path = paths::server_backups_dir(id).join(&b.file);
    {
        let f = std::fs::File::open(&zip_path).map_err(|e| AppError::io("No se pudo abrir la copia recién creada", e))?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| AppError::new("backup", "La copia recién creada está dañada").with_detail(e))?;
        if a.has_world && z.by_name("world/level.dat").is_err() { return Err(AppError::new("backup", "La copia no contiene el mundo: no se sigue.")); }
        for i in 0..z.len() { z.by_index(i).map_err(|e| AppError::new("backup", "La copia recién creada está dañada").with_detail(e))?; }
    }
    steps.push(format!("Copia completa verificada: {} ({:.1} MB)", b.file, b.size_mb));
    let mut record = ChangeRecord { from_mc: cfg.mc_version.clone(), from_loader_version: cfg.loader_version.clone(), from_java_choice: cfg.java_choice, to_mc: to_mc.into(), backup_file: b.file.clone(), mods_aside: None, at: chrono::Local::now().to_rfc3339(), verified: false };
    paths::write_json(&record_path(id), &record)?;

    // 2. Mods: se apartan las versiones viejas (a la Papelera, para poder volver) y se bajan las nuevas.
    let mut target = cfg.clone();
    target.mc_version = to_mc.into();
    target.loader_version = a.loader_version.clone();
    let to_change: Vec<&ModCheck> = a.mods.iter().filter(|m| m.status == "update" || m.status == "missing").collect();
    if !to_change.is_empty() {
        step("Preparando los mods para la versión nueva", 0.2);
        let aside = paths::trash_slot(&format!("{id}-mods-{}", cfg.mc_version));
        std::fs::create_dir_all(&aside).map_err(|e| AppError::io("No se pudo apartar los mods", e))?;
        let dir = mods::mods_dir(id);
        let mut manifest = mods::load_manifest(id);
        for m in &to_change {
            let _ = paths::rename_retry(dir.join(&m.file), aside.join(&m.file));
            manifest.mods.retain(|x| x.file_name != m.file);
        }
        mods::save_manifest(id, &manifest)?;
        record.mods_aside = Some(aside.file_name().unwrap().to_string_lossy().to_string());
        paths::write_json(&record_path(id), &record)?;
        for m in to_change.iter().filter(|m| m.status == "update") {
            let plan = mods::plan_install(src, &target, m.project_id.as_deref().unwrap(), m.target_version_id.as_deref()).await?;
            mods::apply_plan(&target, &plan, |_, _| {}).await?;
        }
        let up = to_change.iter().filter(|m| m.status == "update").count();
        let gone = to_change.len() - up;
        steps.push(format!("Mods: {up} actualizados{}", if gone > 0 { format!(", {gone} apartados por no tener versión para {to_mc}") } else { String::new() }));
    }

    // 3. Componentes de la versión nueva (servidor oficial, loader, Java).
    step("Descargando Minecraft y los componentes necesarios", 0.35);
    let run = paths::server_run_dir(id);
    clear_version_files(&run);
    let mut c2 = instance::load(id)?;
    c2.mc_version = to_mc.into();
    c2.loader_version = a.loader_version.clone();
    if c2.java_choice.map(|j| j < a.java_to).unwrap_or(false) { c2.java_choice = None; }
    c2.installed = false;
    instance::save(&c2)?;
    instance::install(mgr, id.into()).await;
    let c3 = instance::load(id)?;
    if !c3.installed {
        let err = c3.install_error.unwrap_or_else(|| "La instalación no terminó.".into());
        return Ok(ChangeReport { ok: false, backup_file: b.file, steps, error: Some(err), can_rollback: true });
    }
    steps.push(format!("Instalado Minecraft {to_mc}{} con Java {}", a.loader_version.as_ref().map(|v| format!(" · {} {v}", cfg.loader.name())).unwrap_or_default(), c3.java_major));

    // 4. Arranque controlado: convierte el mundo y se verifica que reciba jugadores.
    step("Arrancando para convertir el mundo y comprobar que funcione", 0.75);
    let probe = crate::hardware::Probe::new();
    if let Err(e) = mgr.start(&probe, &c3) { return Ok(ChangeReport { ok: false, backup_file: b.file, steps, error: Some(e.message), can_rollback: true }); }
    let rt = mgr.runtime(id);
    let end = Instant::now() + Duration::from_secs(900);
    let mut verdict: Result<String, String> = Err("no terminó de arrancar a tiempo".into());
    while Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let r = rt.lock().unwrap();
        match (r.status(), r.reach.as_deref()) {
            (RunStatus::Running, Some("ok")) => { verdict = Ok(r.reach_info.as_ref().map(|i| format!("Minecraft {} respondió en {} ms", i.version, i.latency_ms)).unwrap_or_default()); break; }
            (RunStatus::Running, Some("fail")) => { verdict = Err(format!("arrancó pero no recibe conexiones: {}", r.reach_error.clone().unwrap_or_default())); break; }
            (RunStatus::Crashed, _) | (RunStatus::Stopped, _) => { verdict = Err(r.last_error.clone().unwrap_or_else(|| "el servidor se cerró al arrancar".into())); break; }
            _ => {}
        }
    }
    let _ = mgr.stop(id);
    mgr.wait_inactive(id, Duration::from_secs(120));
    match verdict {
        Ok(msg) => {
            steps.push(format!("Arranque de prueba correcto: mundo convertido y {msg}"));
            record.verified = true;
            paths::write_json(&record_path(id), &record)?;
            step("Listo", 1.0);
            Ok(ChangeReport { ok: true, backup_file: b.file, steps, error: None, can_rollback: true })
        }
        Err(e) => Ok(ChangeReport { ok: false, backup_file: b.file, steps, error: Some(format!("El arranque de prueba falló: {e}")), can_rollback: true }),
    }
}

/// Vuelve al estado anterior al último cambio de versión: restaura la copia completa, los mods
/// apartados y la versión, loader y Java anteriores.
pub async fn rollback(mgr: &Manager, id: &str) -> AppResult<()> {
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de volver atrás.")); }
    let rec = last_change(id).ok_or_else(|| AppError::not_found("un cambio de versión para deshacer"))?;
    let cfg = instance::load(id)?;
    backup::restore(mgr, id, &rec.backup_file, &cfg.mc_version)?;
    if let Some(aside) = &rec.mods_aside {
        let src_dir = paths::trash().join(aside);
        let dir = mods::mods_dir(id);
        // Los mods bajados para la versión nueva se apartan; vuelven los anteriores.
        let now_aside = paths::trash_slot(&format!("{id}-mods-{}", rec.to_mc));
        std::fs::create_dir_all(&now_aside).ok();
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() { let _ = paths::rename_retry(e.path(), now_aside.join(e.file_name())); }
        for e in std::fs::read_dir(&src_dir).into_iter().flatten().flatten() { let _ = paths::rename_retry(e.path(), dir.join(e.file_name())); }
    }
    let run = paths::server_run_dir(id);
    clear_version_files(&run);
    let mut c = instance::load(id)?;
    c.mc_version = rec.from_mc.clone();
    c.loader_version = rec.from_loader_version.clone();
    c.java_choice = rec.from_java_choice;
    c.installed = false;
    instance::save(&c)?;
    instance::install(mgr, id.into()).await;
    let c = instance::load(id)?;
    if !c.installed { return Err(AppError::new("install", "Se restauró el mundo, pero no se pudo reinstalar la versión anterior.").with_detail(c.install_error.unwrap_or_default())); }
    let _ = std::fs::remove_file(record_path(id));
    Ok(())
}

/// Qué copiar al duplicar un servidor.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyParts { pub world: bool, pub mods: bool, pub config: bool }

/// Crea un servidor independiente a partir de otro (el original no se toca). Devuelve el id nuevo.
pub async fn duplicate(id: &str, new_name: &str, parts: &CopyParts, total_ram_mb: u64) -> AppResult<String> {
    let src = instance::load(id)?;
    let name = new_name.trim();
    if name.is_empty() { return Err(AppError::invalid("Elegí un nombre para la copia.")); }
    let input = crate::model::CreateServerInput {
        name: name.into(), description: src.description.clone(), mc_version: src.mc_version.clone(), loader: src.loader, memory_mb: src.memory_mb,
        difficulty: src.difficulty.clone(), gamemode: src.gamemode.clone(), max_players: src.max_players, port: free_port(src.port), online_mode: src.online_mode,
        whitelist: src.whitelist, view_distance: src.view_distance, simulation_distance: src.simulation_distance, level_seed: src.level_seed.clone(),
        accept_eula: src.eula_accepted_at.is_some(), import_world_path: None, optimize: vec![],
    };
    let mut cfg = instance::create(input, total_ram_mb).await?;
    let from = paths::server_run_dir(id);
    let to = paths::server_run_dir(&cfg.id);
    std::fs::create_dir_all(&to).map_err(|e| AppError::io("No se pudo crear la copia", e))?;
    let copy = |a: &std::path::Path, b: &std::path::Path| crate::paths::copy_dir(a, b).map(|_| ());
    if parts.world && from.join("world").is_dir() { copy(&from.join("world"), &to.join("world"))?; let _ = std::fs::remove_file(to.join("world").join("session.lock")); }
    if parts.mods {
        if from.join("mods").is_dir() { copy(&from.join("mods"), &to.join("mods"))?; }
        if let Ok(m) = std::fs::read(paths::server_dir(id).join("mods.json")) { let _ = std::fs::write(paths::server_dir(&cfg.id).join("mods.json"), m); }
    }
    if parts.config {
        if from.join("config").is_dir() { copy(&from.join("config"), &to.join("config"))?; }
        for f in ["whitelist.json", "ops.json", "banned-players.json", "banned-ips.json"] { let _ = std::fs::copy(from.join(f), to.join(f)); }
        cfg.motd = src.motd.replace(&src.name, name);
        cfg.extra_properties = src.extra_properties.clone();
        cfg.jvm_profile = src.jvm_profile;
        cfg.extra_jvm_args = src.extra_jvm_args.clone();
    }
    instance::save(&cfg)?;
    Ok(cfg.id)
}

fn free_port(base: u16) -> u16 {
    let used: Vec<u16> = instance::list_configs().iter().map(|c| c.port).collect();
    (base.saturating_add(1)..=base.saturating_add(200)).chain(25565..26000).find(|p| !used.contains(p) && std::net::TcpListener::bind(("0.0.0.0", *p)).is_ok()).unwrap_or(base + 1)
}
