//! Prueba real (aislada en una carpeta temporal) de lo nuevo del Diseño v3:
//! pausa con /tick freeze, guardando, consultas internas ocultas, cuenta regresiva con aviso,
//! reinicio, lectura real de datos de entidad (SNBT de Minecraft 26.2), imágenes del juego
//! descargadas de Mojang y el asistente de reparación de punta a punta (puerto ocupado →
//! reparar con punto de restauración → verificar → deshacer) y con un mod incompatible.
//!
//! Requiere internet (Mojang) y Java 25 en HOST939_TEST_JAVA25 (carpeta con bin/java).
//! cargo test -p host939-core --test v3_features -- --ignored --nocapture --test-threads=1

use host939_core::events::{Events, InstallProgress};
use host939_core::error::{AppError, AppResult};
use host939_core::hardware::Probe;
use host939_core::model::*;
use host939_core::modrinth::{Project, Source, Version};
use host939_core::process::Manager;
use host939_core::{gameassets, instance, mods, nbt, paths, playerdata, repair, snapshots};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Java de prueba: si HOST939_TEST_JAVA25 apunta a un JDK local se usa ese (entornos sin
/// internet); si no, 939 Host descarga Java como lo haría en la PC de un usuario.
#[allow(dead_code)]
fn java_override() {
    if let Ok(j) = std::env::var("HOST939_TEST_JAVA25") {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&j, host939_core::paths::runtimes().join("java-25")).unwrap();
        #[cfg(not(unix))]
        let _ = j;
    }
}

#[derive(Default)]
struct Recorder { lines: Mutex<Vec<String>> }
impl Events for Recorder {
    fn server_changed(&self, _: &str) {}
    fn console_line(&self, _: &str, line: &str) { self.lines.lock().unwrap().push(line.to_string()); }
    fn install_progress(&self, _: &InstallProgress) {}
}

/// Modrinth sin red: la reparación tiene que funcionar igual con lo que se puede hacer local.
struct Offline;
impl Source for Offline {
    async fn version(&self, _: &str) -> AppResult<Version> { Err(AppError::invalid("sin red")) }
    async fn project_versions(&self, _: &str, _: &str, _: &str) -> AppResult<Vec<Version>> { Ok(vec![]) }
    async fn projects(&self, _: &[String]) -> AppResult<Vec<Project>> { Ok(vec![]) }
    async fn versions_by_sha1(&self, _: &[String]) -> AppResult<HashMap<String, Version>> { Ok(HashMap::new()) }
}

fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end { if f() { return true; } std::thread::sleep(Duration::from_millis(200)); }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn design_v3_features() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    java_override();
    let rec = Arc::new(Recorder::default());
    let mgr = Arc::new(Manager::new(rec.clone()));
    let probe = Probe::new();

    let input = CreateServerInput {
        name: "Prueba v3".into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Vanilla,
        memory_mb: 1536, difficulty: "peaceful".into(), gamemode: "survival".into(), max_players: 4, port: 25631,
        online_mode: true, whitelist: true, view_distance: 3, simulation_distance: 3, level_seed: "939".into(),
        accept_eula: true, import_world_path: None,
    };
    let cfg = instance::create(input, 8192).await.expect("crear");
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "instalación: {:?}", cfg.install_error);
    let rt = mgr.runtime(&cfg.id);
    let _guard = Guard(mgr.clone(), cfg.id.clone());

    // Etapas de arranque en palabras y estado final.
    mgr.start(&probe, &cfg).expect("iniciar");
    let mut phases = vec![];
    assert!(wait_for(Duration::from_secs(400), || {
        let r = rt.lock().unwrap();
        if let Some(p) = &r.phase { if phases.last() != Some(p) { phases.push(p.clone()); } }
        r.status() == RunStatus::Running
    }), "no llegó a Running");
    assert!(rt.lock().unwrap().phase.is_none());
    println!("✔ en línea · etapas vistas: {phases:?}");

    // Consulta interna: no aparece en la consola.
    let before = mgr.console(&cfg.id).len();
    let line = mgr.query(&cfg.id, "list", &["players online"], Duration::from_secs(5)).expect("consulta");
    assert!(line.contains("players online"));
    assert!(!mgr.console(&cfg.id)[before..].iter().any(|l| l.contains("players online") || l.contains("> list")), "la consulta se filtró a la consola");
    println!("✔ consulta interna oculta: «{}»", line.trim());

    // Pausa real con /tick freeze y reanudación.
    mgr.pause(&cfg).expect("pausar");
    assert!(rt.lock().unwrap().paused);
    let q = mgr.query(&cfg.id, "tick query", &["The game is frozen", "Target tick rate", "The game is running"], Duration::from_secs(5)).unwrap();
    println!("✔ pausado · tick query: «{}»", q.trim());
    assert!(q.contains("frozen"), "debería decir frozen: {q}");
    mgr.resume(&cfg.id).expect("reanudar");
    assert!(!rt.lock().unwrap().paused);
    println!("✔ reanudado");

    // Guardando: save-all enciende y apaga el estado.
    let n0 = mgr.console(&cfg.id).len();
    mgr.send_command(&cfg.id, "save-all flush").unwrap();
    let mut saw_saving = false;
    // En un mundo chico el guardado puede durar menos que el intervalo de consulta: se acepta
    // haber visto el estado o la confirmación de Minecraft, y que al final no quede «Guardando».
    let saved = |m: &Manager| m.console(&cfg.id).iter().skip(n0.saturating_sub(5)).any(|l| l.contains("Saved the game") || l.contains("All dimensions are saved"));
    assert!(wait_for(Duration::from_secs(30), || { let s = rt.lock().unwrap().saving; saw_saving |= s; (saw_saving || saved(&mgr)) && !s }), "no se vio Guardando → listo:\n{}", mgr.console(&cfg.id).iter().skip(n0.saturating_sub(5)).cloned().collect::<Vec<_>>().join("\n"));
    println!("✔ estado «Guardando» detectado y terminado");

    // Datos reales de una entidad (SNBT de 26.2) con el lector de jugadores.
    // Desde 1.21.9 no hay chunks de spawn siempre cargados: se fuerza la carga para la prueba.
    mgr.query(&cfg.id, "forceload add 0 0", &["Marked chunk", "already", "forceloaded"], Duration::from_secs(10)).expect("forceload");
    std::thread::sleep(Duration::from_millis(1500));
    let summoned = mgr.query(&cfg.id, "summon minecraft:armor_stand 0 100 0 {Tags:[\"t939\"],NoGravity:1b}", &["Summoned", "not loaded", "Unable", "Invalid"], Duration::from_secs(5)).unwrap();
    assert!(summoned.contains("Summoned"), "summon: {summoned}");
    let reply = mgr.query(&cfg.id, "data get entity @e[tag=t939,limit=1]", &["has the following entity data", "No entity was found"], Duration::from_secs(5)).unwrap();
    let snbt = &reply[reply.find("entity data: ").unwrap_or_else(|| panic!("respuesta: {reply}")) + 13..];
    let v = nbt::parse_snbt(snbt).expect("SNBT real de Minecraft");
    let d = playerdata::from_value(&v);
    assert_eq!(d.pos.map(|p| p[1].round() as i64), Some(100));
    assert!(d.health.is_some());
    println!("✔ SNBT real leído: salud {:?}, posición {:?}, mirando {:?}", d.health, d.pos, d.facing);
    assert!(!mgr.console(&cfg.id).iter().any(|l| l.contains("has the following entity data")));

    // Cuenta regresiva con aviso en el chat (3 s) y cancelación.
    let m2 = mgr.clone();
    let id2 = cfg.id.clone();
    let h = std::thread::spawn(move || m2.countdown(&id2, 10, "stop"));
    assert!(wait_for(Duration::from_secs(3), || rt.lock().unwrap().countdown.is_some()));
    std::thread::sleep(Duration::from_millis(1200));
    assert!(mgr.cancel_countdown(&cfg.id));
    assert!(!h.join().unwrap(), "cancelada debe devolver false");
    assert!(wait_for(Duration::from_secs(5), || { let log = rec.lines.lock().unwrap().join("\n"); log.contains("El servidor se detiene en 10 segundos") && log.contains("Cancelado") }), "faltan los avisos en el chat");
    let m3 = mgr.clone();
    let id3 = cfg.id.clone();
    assert!(std::thread::spawn(move || m3.countdown(&id3, 3, "restart")).join().unwrap());
    println!("✔ cuenta regresiva: avisos en el chat, cancelación y fin");

    // Reinicio: Reiniciando → detenido → Done limpia el estado.
    mgr.set_restarting(&cfg.id, true);
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(100)));
    assert!(rt.lock().unwrap().restarting, "durante el reinicio sigue en Reiniciando");
    mgr.start(&probe, &cfg).unwrap();
    assert!(wait_for(Duration::from_secs(300), || rt.lock().unwrap().status() == RunStatus::Running));
    assert!(!rt.lock().unwrap().restarting);
    println!("✔ reinicio completo");
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(100)));

    // Imágenes del juego: descarga verificada de Mojang, extracción mínima y nombres en español.
    gameassets::ensure("26.2", |_, _| {}).await.expect("imágenes del juego");
    assert!(gameassets::ready("26.2"));
    for k in ["item:minecraft:diamond_sword", "item:compass", "item:torch", "hud:heart/full", "hud:food_half", "slot:helmet", "effect:speed"] {
        assert!(gameassets::resolve("26.2", k).is_some(), "falta {k}");
    }
    let names = gameassets::names("26.2");
    println!("✔ imágenes del juego listas · «{}», «{}»", names["item:diamond_sword"], names["effect:speed"]);
    assert_eq!(names["item:diamond_sword"], "Espada de diamante");
    let size: u64 = walk(&gameassets::dir("26.2"));
    assert!(size < 40_000_000, "la caché debería ser chica: {size}");
    assert!(!std::fs::read_dir(paths::cache()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".jar") || e.file_name().to_string_lossy().ends_with(".part")), "el cliente descargado debe borrarse");
    println!("✔ caché de imágenes: {:.1} MB, sin el jar del cliente", size as f64 / 1e6);

    // Reparación: puerto ocupado → diagnóstico → reparar con punto previo → verificar → deshacer.
    let blocker = std::net::TcpListener::bind(("0.0.0.0", cfg.port)).unwrap();
    let diag = repair::analyze(&Offline, &mgr, &cfg, 4096, false).await;
    let p = diag.problems.iter().find(|p| p.kind == "port").expect("debería detectar el puerto ocupado");
    let fix = p.fix.clone().expect("con solución");
    println!("✔ diagnóstico: «{}» → {}", p.title, fix.summary);
    let snaps_before = snapshots::list(&cfg.id).len();
    let rec_ = repair::apply(&Offline, &mgr, &cfg, &fix.actions, |_, _| {}).await.expect("reparar");
    let fixed = instance::load(&cfg.id).unwrap();
    assert_ne!(fixed.port, cfg.port);
    assert_eq!(snapshots::list(&cfg.id).len(), snaps_before + 1, "falta el punto «Antes de reparar»");
    assert!(snapshots::list(&cfg.id).iter().any(|s| s.label == "Antes de reparar"));
    mgr.start(&probe, &fixed).expect("arrancar reparado");
    assert!(wait_for(Duration::from_secs(300), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó después de reparar");
    println!("✔ reparado ({:?}) y verificado: arrancó en el puerto {}", rec_.applied, fixed.port);
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(100)));
    repair::undo(&mgr, &cfg.id).expect("deshacer");
    assert_eq!(instance::load(&cfg.id).unwrap().port, cfg.port);
    drop(blocker);
    println!("✔ deshacer: puerto original restaurado");

    // Reparación de mods (sin red): mod de Fabric para otra versión y mod de cliente.
    let mut fcfg = instance::load(&cfg.id).unwrap();
    fcfg.loader = Loader::Fabric;
    let dir = mods::mods_dir(&cfg.id);
    std::fs::create_dir_all(&dir).unwrap();
    fake_jar(&dir.join("viejo.jar"), r#"{"id":"viejo","name":"Mod Viejo","version":"1.0","depends":{"minecraft":"~1.20.1"}}"#);
    fake_jar(&dir.join("cliente.jar"), r#"{"id":"zoom","name":"Zoom","version":"2.0","environment":"client"}"#);
    fake_jar(&dir.join("ok.jar"), r#"{"id":"bien","name":"Bien","version":"1.0","depends":{"minecraft":">=26.2","falta":"*"}}"#);
    let diag = repair::analyze(&Offline, &mgr, &fcfg, 4096, false).await;
    let kinds: Vec<&str> = diag.problems.iter().map(|p| p.kind.as_str()).collect();
    println!("✔ mods analizados: {kinds:?}");
    assert!(kinds.contains(&"mc_version") && kinds.contains(&"client_only") && kinds.contains(&"missing_dependency"));
    let acts: Vec<repair::Action> = diag.problems.iter().filter(|p| p.kind != "missing_dependency").filter_map(|p| p.fix.clone()).flat_map(|f| f.actions).collect();
    repair::apply(&Offline, &mgr, &fcfg, &acts, |_, _| {}).await.expect("apartar mods");
    let left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["ok.jar".to_string()], "solo debe quedar el mod compatible");
    repair::undo(&mgr, &cfg.id).expect("deshacer mods");
    println!("✔ mods incompatibles apartados a la Papelera y deshacer disponible");
}

/// Si la prueba falla a mitad de camino, no deja un servidor de prueba encendido.
struct Guard(Arc<Manager>, String);
impl Drop for Guard { fn drop(&mut self) { let _ = self.0.kill(&self.1); self.0.wait_inactive(&self.1, Duration::from_secs(10)); } }

fn fake_jar(path: &std::path::Path, json: &str) {
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(json.as_bytes()).unwrap();
    z.finish().unwrap();
}
fn walk(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| if e.path().is_dir() { walk(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()).unwrap_or(0)
}
