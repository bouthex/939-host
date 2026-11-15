//! Servidores: creación, instalación, edición, importación de mundos y borrado seguro.

use crate::error::{AppError, AppResult};
use crate::hardware::Probe;
use crate::identity;
use crate::java;
use crate::meta;
use crate::model::*;
use crate::net;
use crate::paths;
use crate::events::InstallProgress;
use crate::process::Manager;
use crate::properties;
use std::path::{Path, PathBuf};


fn config_path(id: &str) -> PathBuf { paths::server_dir(id).join("instance.json") }

pub fn load(id: &str) -> AppResult<ServerConfig> {
    if id.contains(['/', '\\', '.']) { return Err(AppError::invalid("Identificador inválido")); }
    let p = config_path(id);
    if !p.exists() { return Err(AppError::not_found("el servidor")); }
    paths::read_json(&p)
}

pub fn save(cfg: &ServerConfig) -> AppResult<()> { paths::write_json(&config_path(&cfg.id), cfg) }

pub fn list_configs() -> Vec<ServerConfig> {
    let mut out: Vec<ServerConfig> = std::fs::read_dir(paths::servers())
        .map(|rd| rd.flatten().filter_map(|e| paths::read_json::<ServerConfig>(&e.path().join("instance.json")).ok()).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    out
}

pub fn view(mgr: &Manager, probe: &Probe, cfg: ServerConfig, with_size: bool) -> ServerView {
    let rt = mgr.runtime(&cfg.id);
    let r = rt.lock().unwrap();
    let memory_used_mb = r.pid.and_then(|pid| crate::hardware::process_memory_mb(probe, pid));
    let world_size_mb = if with_size { paths::dir_size(&paths::server_run_dir(&cfg.id).join("world")) as f64 / 1_048_576.0 } else { 0.0 };
    ServerView {
        status: r.status(),
        players: r.players.iter().cloned().collect(),
        started_at: r.started_at.map(|t| t.to_rfc3339()),
        memory_used_mb,
        mspt: r.mspt,
        mspt_p95: r.mspt_p95,
        last_error: r.last_error.clone().or_else(|| cfg.install_error.clone()),
        needs_restart: r.needs_restart,
        installing: r.installing,
        world_size_mb,
        paused: r.paused,
        saving: r.saving || r.busy.is_some(),
        restarting: r.restarting,
        phase: r.phase.clone(),
        countdown: r.countdown.clone(),
        pause_supported: crate::process::supports_tick(&cfg.mc_version),
        config: cfg,
    }
}

fn slugify(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let s: String = s.chars().take(32).collect();
    if s.is_empty() { "servidor".into() } else { s }
}

fn random_suffix() -> String {
    let mut b = [0u8; 2];
    let _ = getrandom::getrandom(&mut b);
    hex::encode(b)
}

const DIFFICULTIES: &[&str] = &["peaceful", "easy", "normal", "hard"];
const GAMEMODES: &[&str] = &["survival", "creative", "adventure", "spectator"];

pub fn validate(cfg: &ServerConfig, total_ram_mb: u64) -> AppResult<()> {
    let name = cfg.name.trim();
    if name.is_empty() || name.chars().count() > 40 { return Err(AppError::invalid("El nombre debe tener entre 1 y 40 caracteres.")); }
    if cfg.memory_mb < 1024 { return Err(AppError::invalid("La RAM mínima para un servidor es 1024 MB.")); }
    let max = total_ram_mb.saturating_sub(1536).max(1024);
    if cfg.memory_mb as u64 > max { return Err(AppError::invalid(format!("Esta PC tiene {total_ram_mb} MB; como máximo se pueden asignar {max} MB para dejar margen a Windows."))); }
    if cfg.port < 1024 { return Err(AppError::invalid("El puerto debe estar entre 1024 y 65535.")); }
    if !(1..=500).contains(&cfg.max_players) { return Err(AppError::invalid("Los jugadores máximos deben estar entre 1 y 500.")); }
    if !(2..=32).contains(&cfg.view_distance) || !(2..=32).contains(&cfg.simulation_distance) { return Err(AppError::invalid("Las distancias deben estar entre 2 y 32 chunks.")); }
    if !DIFFICULTIES.contains(&cfg.difficulty.as_str()) { return Err(AppError::invalid("Dificultad inválida.")); }
    if !GAMEMODES.contains(&cfg.gamemode.as_str()) { return Err(AppError::invalid("Modo de juego inválido.")); }
    if cfg.sync_interval_min < 1 || cfg.sync_interval_min > 240 || cfg.restore_interval_min < 5 || cfg.restore_interval_min > 1440 {
        return Err(AppError::invalid("Intervalos fuera de rango (relevo: 1–240 min, restauración: 5–1440 min)."));
    }
    if cfg.spawn_protection > 1000 { return Err(AppError::invalid("La protección del spawn debe estar entre 0 y 1000 bloques.")); }
    for other in list_configs() {
        if other.id != cfg.id && other.port == cfg.port {
            return Err(AppError::invalid(format!("El puerto {} ya lo usa el servidor «{}».", cfg.port, other.name)));
        }
    }
    Ok(())
}

/// Crea el servidor (carpetas, identidad, mundo importado). La instalación se lanza aparte con `install`.
pub async fn create(input: CreateServerInput, total_ram_mb: u64) -> AppResult<ServerConfig> {
    if !input.accept_eula {
        return Err(AppError::invalid("Para crear un servidor hay que aceptar el EULA de Minecraft."));
    }
    let import = match &input.import_world_path {
        Some(p) if !p.trim().is_empty() => Some(validate_world_source(Path::new(p))?),
        _ => None,
    };
    let mut loader_version = None;
    if input.loader != Loader::Vanilla {
        let support = match meta::loader_support(input.loader, &input.mc_version).await {
            Ok(s) => s,
            Err(e) if input.loader == Loader::Fabric => meta::fabric_support_maven(&input.mc_version).await
                .map(|f| meta::LoaderSupport { supported: f.supported, loader_version: f.loader_version, unverified: false }).map_err(|_| e)?,
            Err(e) => return Err(e),
        };
        if !support.supported {
            return Err(AppError::invalid(format!("{} todavía no tiene una versión para Minecraft {}.", input.loader.name(), input.mc_version)));
        }
        loader_version = support.loader_version;
    }
    let mut id = format!("{}-{}", slugify(&input.name), random_suffix());
    while paths::server_dir(&id).exists() { id = format!("{}-{}", slugify(&input.name), random_suffix()); }

    let now = chrono::Local::now().to_rfc3339();
    let mut cfg = ServerConfig {
        id: id.clone(),
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        mc_version: input.mc_version.clone(),
        loader: input.loader,
        loader_version,
        java_major: 0,
        memory_mb: input.memory_mb,
        jvm_profile: JvmProfile::Balanced,
        extra_jvm_args: String::new(),
        port: input.port,
        max_players: input.max_players,
        motd: format!("{} · 939 Host", input.name.trim()),
        online_mode: input.online_mode,
        whitelist: input.whitelist,
        difficulty: input.difficulty,
        gamemode: input.gamemode,
        hardcore: false,
        pvp: true,
        level_seed: input.level_seed.trim().to_string(),
        level_type: "minecraft:normal".into(),
        view_distance: input.view_distance,
        simulation_distance: input.simulation_distance,
        spawn_protection: 16,
        allow_flight: false,
        enable_command_block: false,
        sync_interval_min: 15,
        restore_interval_min: 15,
        installed: false,
        install_error: None,
        eula_accepted_at: Some(now.clone()),
        created_at: now,
        identity: ServerIdentity::default(),
        extra_properties: Default::default(),
    };
    validate(&cfg, total_ram_mb)?;
    std::fs::create_dir_all(paths::server_run_dir(&id)).map_err(|e| AppError::io("No se pudo crear la carpeta del servidor", e))?;
    std::fs::create_dir_all(paths::server_backups_dir(&id)).map_err(|e| AppError::io("No se pudo crear la carpeta de backups", e))?;
    cfg.identity = identity::create_server_identity(&id, &cfg.name)?;
    save(&cfg)?;

    if let Some(src) = import {
        let dst = paths::server_run_dir(&id).join("world");
        tokio::task::spawn_blocking(move || paths::copy_dir(&src, &dst))
            .await
            .map_err(|e| AppError::new("internal", "Falló la importación del mundo").with_detail(e))??;
    }

    Ok(cfg)
}

/// Descarga e instala todo lo necesario. Se puede reintentar si falló.
pub async fn install(mgr: &Manager, id: String) {
    let ev = mgr.events.clone();
    let rt = mgr.runtime(&id);
    {
        let mut r = rt.lock().unwrap();
        if r.installing { return; }
        r.installing = true;
    }
    mgr.changed(&id);
    let report = |message: &str, fraction: f64| {
        ev.install_progress(&InstallProgress { id: id.clone(), message: message.to_string(), fraction, done: false, error: None });
    };
    let result = async {
        let mut cfg = load(&id)?;
        report("Consultando la versión en Mojang", 0.02);
        let dl = meta::server_download(&cfg.mc_version).await?;
        cfg.java_major = dl.java_major;
        save(&cfg)?;
        java::ensure(dl.java_major, |msg, f| report(msg, 0.05 + f * 0.55)).await?;

        let run = paths::server_run_dir(&id);
        // En un traspaso, la PC nueva instala antes de recibir el mundo: la carpeta puede no existir.
        std::fs::create_dir_all(&run).map_err(|e| AppError::io("No se pudo crear la carpeta del servidor", e))?;
        if !cfg.loader.uses_installer() {
            let cached = paths::cache().join("jars").join(format!("{}.jar", dl.sha1));
            if !cached.exists() || net::sha1_file(&cached).map(|h| h != dl.sha1).unwrap_or(true) {
                net::download(&dl.url, &cached, net::Hash::Sha1(&dl.sha1), |d, t| {
                    if let Some(t) = t { report("Descargando el servidor de Minecraft", 0.6 + 0.3 * d as f64 / t as f64); }
                }).await?;
            }
            std::fs::copy(&cached, run.join("server.jar")).map_err(|e| AppError::io("No se pudo copiar el servidor", e))?;
        }

        if cfg.loader.uses_installer() {
            let name = cfg.loader.name();
            let lv = cfg.loader_version.clone().ok_or_else(|| AppError::invalid(format!("Falta la versión de {name}")))?;
            let url = meta::installer_url(cfg.loader, &cfg.mc_version, &lv).ok_or_else(|| AppError::invalid("Loader sin instalador"))?;
            let installer = paths::cache().join("installers").join(format!("{}-{}-{}-installer.jar", name.to_lowercase(), cfg.mc_version, lv));
            if !installer.exists() {
                report(&format!("Descargando el instalador de {name}"), 0.62);
                net::download(&url, &installer, net::Hash::None, |_, _| {}).await?;
            }
            report(&format!("Instalando {name} (descarga sus librerías; puede tardar unos minutos)"), 0.7);
            let java = java::installed_java(dl.java_major).ok_or_else(|| AppError::new("java", "Falta Java para el instalador."))?;
            let log_path = run.join("logs").join("instalador.log");
            std::fs::create_dir_all(log_path.parent().unwrap()).ok();
            let installer2 = installer.clone();
            let run2 = run.clone();
            let out = tokio::task::spawn_blocking(move || {
                let mut c = std::process::Command::new(java);
                c.current_dir(&run2).arg("-jar").arg(&installer2).arg("--installServer");
                #[cfg(windows)]
                { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
                c.output()
            }).await.map_err(|e| AppError::new("install", "El instalador se interrumpió").with_detail(e))?
              .map_err(|e| AppError::io("No se pudo ejecutar el instalador", e))?;
            let mut log = out.stdout.clone();
            log.extend_from_slice(&out.stderr);
            let _ = std::fs::write(&log_path, &log);
            if !out.status.success() || crate::process::launch_target(&cfg).is_none() {
                let tail: String = String::from_utf8_lossy(&log).lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
                return Err(AppError::new("install", format!("El instalador de {name} no terminó bien.")).with_detail(tail));
            }
            // El instalador deja su propio run.bat y user_jvm_args.txt: 939 Host arranca con sus propios ajustes.
            let _ = std::fs::remove_file(run.join("installer.jar.log"));
        }

        if cfg.loader == Loader::Fabric {
            report("Instalando Fabric", 0.92);
            let lv = cfg.loader_version.clone().ok_or_else(|| AppError::invalid("Falta la versión de Fabric"))?;
            let official = async {
                let url = meta::fabric_server_launcher_url(&cfg.mc_version, &lv).await?;
                net::download(&url, &run.join("fabric-server-launch.jar"), net::Hash::None, |_, _| {}).await?;
                paths::write_atomic(&run.join("fabric-server-launcher.properties"), b"serverJar=server.jar\n")
            }.await;
            if let Err(e) = official {
                // Plan B: si el servicio de Fabric no responde, se arma desde su repositorio Maven.
                report("Instalando Fabric desde su repositorio", 0.93);
                let _ = std::fs::remove_file(run.join("fabric-server-launch.jar"));
                meta::fabric_install_maven(&run, &cfg.mc_version, &lv).await.map_err(|e2| AppError::new("install", "No se pudo instalar Fabric.").with_detail(format!("{} / {}", e.message, e2.message)))?;
            }
        }

        report("Escribiendo la configuración", 0.97);
        properties::write_for(&cfg)?;
        let eula = format!("# Aceptado en 939 Host el {}\n# https://aka.ms/MinecraftEULA\neula=true\n", cfg.eula_accepted_at.clone().unwrap_or_default());
        paths::write_atomic(&run.join("eula.txt"), eula.as_bytes())?;
        cfg.installed = true;
        cfg.install_error = None;
        save(&cfg)?;
        Ok::<_, AppError>(())
    }.await;

    rt.lock().unwrap().installing = false;
    match result {
        Ok(()) => {
            ev.install_progress(&InstallProgress { id: id.clone(), message: "Listo para iniciar".into(), fraction: 1.0, done: true, error: None });
        }
        Err(e) => {
            if let Ok(mut cfg) = load(&id) { cfg.install_error = Some(e.message.clone()); let _ = save(&cfg); }
            ev.install_progress(&InstallProgress { id: id.clone(), message: "La instalación falló".into(), fraction: 0.0, done: true, error: Some(e.message.clone()) });
        }
    }
    mgr.changed(&id);
}

/// Guarda cambios de configuración. Versión y loader no se cambian desde acá.
pub fn update(mgr: &Manager, mut patch: ServerConfig, total_ram_mb: u64) -> AppResult<ServerConfig> {
    let current = load(&patch.id)?;
    patch.mc_version = current.mc_version.clone();
    patch.loader = current.loader;
    patch.loader_version = current.loader_version.clone();
    patch.java_major = current.java_major;
    patch.installed = current.installed;
    patch.install_error = current.install_error.clone();
    patch.created_at = current.created_at.clone();
    patch.identity = current.identity.clone();
    patch.eula_accepted_at = current.eula_accepted_at.clone();
    patch.name = patch.name.trim().to_string();
    validate(&patch, total_ram_mb)?;
    save(&patch)?;
    if patch.installed { properties::write_for(&patch)?; }
    let rt = mgr.runtime(&patch.id);
    let mut r = rt.lock().unwrap();
    if r.is_active() { r.needs_restart = true; }
    Ok(patch)
}

/// Borrado seguro: el servidor va a la Papelera de 939 Host, no se elimina del disco.
pub fn delete(mgr: &Manager, id: &str, confirm_name: &str) -> AppResult<()> {
    let cfg = load(id)?;
    if confirm_name.trim() != cfg.name {
        return Err(AppError::invalid("El nombre escrito no coincide. No se borró nada."));
    }
    if mgr.runtime(id).lock().unwrap().is_active() {
        return Err(AppError::busy("Detené el servidor antes de borrarlo."));
    }
    let dst = paths::trash_slot(id);
    std::fs::rename(paths::server_dir(id), &dst).map_err(|e| AppError::io("No se pudo mover el servidor a la Papelera", e))?;
    Ok(())
}

/// Comprueba que la carpeta sea un mundo de Minecraft y que no esté abierto en el juego.
pub fn validate_world_source(src: &Path) -> AppResult<PathBuf> {
    if !src.join("level.dat").exists() {
        return Err(AppError::invalid("La carpeta elegida no es un mundo de Minecraft (no tiene level.dat)."));
    }
    if paths::root().starts_with(src) || src.starts_with(paths::root()) {
        return Err(AppError::invalid("Elegí un mundo que no esté dentro de las carpetas de 939 Host."));
    }
    // Minecraft bloquea session.lock mientras el mundo está abierto: leerlo falla con
    // "violación de bloqueo". Solo se lee; el mundo original nunca se modifica.
    let lock = src.join("session.lock");
    if lock.exists() {
        if let Err(e) = std::fs::read(&lock) {
            if matches!(e.raw_os_error(), Some(32) | Some(33)) {
                return Err(AppError::busy("Ese mundo está abierto en Minecraft. Salí al menú principal y volvé a intentar; así la copia queda consistente."));
            }
        }
    }
    Ok(src.to_path_buf())
}

/// Reemplaza el mundo del servidor por una copia de otro mundo (el actual queda en la Papelera).
pub fn import_world(mgr: &Manager, id: &str, src: &Path) -> AppResult<u64> {
    let _ = load(id)?;
    if mgr.runtime(id).lock().unwrap().is_active() { return Err(AppError::busy("Detené el servidor antes de importar un mundo.")); }
    let src = validate_world_source(src)?;
    let world = paths::server_run_dir(id).join("world");
    if world.exists() {
        std::fs::rename(&world, paths::trash_slot(&format!("{id}-world"))).map_err(|e| AppError::io("No se pudo apartar el mundo actual", e))?;
    }
    paths::copy_dir(&src, &world)
}
