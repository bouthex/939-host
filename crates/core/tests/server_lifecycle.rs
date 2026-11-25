//! Prueba real de punta a punta en una carpeta temporal aislada:
//! crear → instalar (descarga verificada de Mojang) → iniciar → métricas → backup en caliente →
//! detener → protecciones (doble inicio, puerto ocupado) → restaurar → borrar con confirmación.
//!
//! Requiere internet (Mojang) y Java 25 en HOST939_TEST_JAVA25 (carpeta con bin/java).
//! Se ejecuta con: cargo test -p host939-core --test server_lifecycle -- --ignored --nocapture

use host939_core::events::{Events, InstallProgress};
use host939_core::hardware::Probe;
use host939_core::model::*;
use host939_core::process::Manager;
use host939_core::{backup, instance, paths, snapshots};
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
struct Recorder {
    lines: Mutex<Vec<String>>,
    progress: Mutex<Vec<String>>,
}
impl Events for Recorder {
    fn server_changed(&self, _: &str) {}
    fn console_line(&self, _: &str, line: &str) { self.lines.lock().unwrap().push(line.to_string()); }
    fn install_progress(&self, p: &InstallProgress) {
        self.progress.lock().unwrap().push(format!("{} {:.0}% {:?}", p.message, p.fraction * 100.0, p.error));
    }
}

fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() { return true; }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn full_server_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    java_override();

    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();

    // 1. Crear
    let input = CreateServerInput {
        name: "Prueba 939".into(), description: "aislada".into(), mc_version: "26.2".into(), loader: Loader::Vanilla,
        memory_mb: 2048, difficulty: "hard".into(), gamemode: "survival".into(), max_players: 4, port: 25611,
        online_mode: true, whitelist: true, view_distance: 4, simulation_distance: 4, level_seed: "939".into(),
        accept_eula: true, import_world_path: None, optimize: vec![],
    };
    let cfg = instance::create(input, 8192).await.expect("crear");
    assert!(cfg.identity.access_code.starts_with("PRUEBA-939 · "), "código: {}", cfg.identity.access_code);
    println!("✔ creado {} · código {}", cfg.id, cfg.identity.access_code);

    // 2. Instalar
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "instalación: {:?} / {:?}", cfg.install_error, rec.progress.lock().unwrap());
    assert_eq!(cfg.java_major, 25);
    let props = std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("server.properties")).unwrap();
    assert!(props.contains("server-port=25611") && props.contains("simulation-distance=4") && props.contains("white-list=true"));
    println!("✔ instalado (Java {}), server.properties correcto", cfg.java_major);

    // 3. Iniciar y esperar "Done"
    mgr.start(&probe, &cfg).expect("iniciar");
    assert!(mgr.start(&probe, &cfg).is_err(), "no debe permitir doble inicio");
    let rt = mgr.runtime(&cfg.id);
    assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running), "no llegó a Running");
    assert!(paths::server_run_dir(&cfg.id).join(".939host.lock").exists(), "falta el lock");
    println!("✔ en línea; doble inicio bloqueado; lock presente");

    // 4. Métricas (tick query cada 20 s, salida oculta de la consola)
    assert!(wait_for(Duration::from_secs(60), || rt.lock().unwrap().mspt.is_some()), "sin MSPT");
    let hidden = !rec.lines.lock().unwrap().iter().any(|l| l.contains("Target tick rate"));
    let (mspt, p95) = { let r = rt.lock().unwrap(); (r.mspt, r.mspt_p95) };
    println!("✔ MSPT {mspt:?} p95 {p95:?}; salida de tick query oculta: {hidden}");
    assert!(hidden);
    mgr.send_command(&cfg.id, "say hola desde 939 Host").unwrap();

    // 5. Backup en caliente (save-off → save-all flush → copia → save-on)
    let info = backup::create(&mgr, &cfg.id, "en caliente", "manual", &cfg.mc_version).expect("backup");
    let zip = paths::server_backups_dir(&cfg.id).join(&info.file);
    {
        // Se cierra enseguida: en Windows un archivo abierto impide mover la carpeta después.
        let mut z = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
        assert!(z.by_name("world/level.dat").is_ok());
    }
    let log = mgr.console(&cfg.id).join("\n");
    assert!(log.contains("> save-off") && log.contains("> save-on") && log.contains("Saved the game"));
    println!("✔ backup en caliente {} ({:.1} MB) con save-off/save-on", info.file, info.size_mb);

    // 5b. Puntos de restauración incrementales con el servidor encendido
    let s1 = snapshots::take(&mgr, &cfg.id, "", "auto", &cfg.mc_version).expect("punto 1");
    mgr.send_command(&cfg.id, "time set night").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let s2 = snapshots::take(&mgr, &cfg.id, "", "auto", &cfg.mc_version).expect("punto 2");
    let st = snapshots::stats(&cfg.id);
    println!("✔ puntos incrementales: 1º {:.1} MB nuevos (pausa {} ms), 2º {:.1} KB nuevos (pausa {} ms); almacén {:.1} MB para {:.1} MB lógicos",
        s1.new_bytes as f64 / 1e6, s1.pause_ms, s2.new_bytes as f64 / 1e3, s2.pause_ms, st.store_bytes as f64 / 1e6, st.logical_bytes as f64 / 1e6);
    assert!(s2.new_bytes < s1.new_bytes / 4, "el segundo punto debe ser mucho más chico");
    assert!(mgr.console(&cfg.id).iter().filter(|l| l.contains("> save-on")).count() >= 3);

    // 6. Detener ordenadamente
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(100)));
    let (st, last_err) = { let r = rt.lock().unwrap(); (r.status(), r.last_error.clone()) };
    assert_eq!(st, RunStatus::Stopped, "debería ser detención limpia: {last_err:?}");
    assert!(!paths::server_run_dir(&cfg.id).join(".939host.lock").exists(), "el lock debe borrarse");
    println!("✔ detenido limpio, lock liberado");

    // 7. Puerto ocupado por otro programa que escucha en todas las direcciones (antes la
    //    comprobación en 127.0.0.1 lo daba por libre, Java fallaba al abrirlo y «Reparar» decía que
    //    no había problema).
    let blocker = std::net::TcpListener::bind(("0.0.0.0", 25611)).unwrap();
    // 7a. Con acceso desde la red local, el número importa: no se cambia solo, se explica.
    let mut st = host939_core::settings::load();
    st.lan_access = true;
    host939_core::settings::save(&st).unwrap();
    let err = mgr.start(&probe, &cfg).unwrap_err();
    assert_eq!(err.code, "port");
    assert!(err.message.contains("Reparar"), "{}", err.message);
    println!("✔ con acceso de la red local: no cambia el puerto solo ({})", err.message);
    // 7b. Predeterminado (solo esta PC): pasa solo a un puerto libre y enciende de verdad.
    st.lan_access = false;
    host939_core::settings::save(&st).unwrap();
    mgr.start(&probe, &cfg).expect("enciende en otro puerto");
    let moved = instance::load(&cfg.id).unwrap();
    assert_ne!(moved.port, 25611);
    assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running), "no encendió en el puerto nuevo");
    let props = std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("server.properties")).unwrap();
    assert!(props.contains(&format!("server-port={}", moved.port)));
    assert!(mgr.console(&cfg.id).iter().any(|l| l.contains(&format!("pasó al puerto {}", moved.port))), "se explica el cambio");
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(100)));
    drop(blocker);
    let mut back = moved.clone();
    back.port = 25611;
    instance::save(&back).unwrap();
    println!("✔ puerto ocupado por otro programa: el servidor pasó solo al {} y encendió", moved.port);

    // 8. Restaurar (crea backup de seguridad y aparta el mundo actual)
    backup::restore(&mgr, &cfg.id, &info.file, &cfg.mc_version).expect("restaurar");
    let list = backup::list(&cfg.id);
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|b| b.kind == "seguridad"));
    assert!(paths::server_run_dir(&cfg.id).join("world/level.dat").exists());
    assert!(std::fs::read_dir(paths::trash()).unwrap().count() >= 1);
    println!("✔ restaurado con backup de seguridad previo");

    // 8b. Restaurar un punto incremental
    snapshots::restore(&mgr, &cfg.id, &s1.id, &cfg.mc_version).expect("restaurar punto");
    assert!(paths::server_run_dir(&cfg.id).join("world/level.dat").exists());
    assert!(snapshots::list(&cfg.id).iter().any(|s| s.kind == "seguridad"));
    println!("✔ punto incremental restaurado");

    // 9. Borrado: nombre incorrecto no borra; correcto mueve a la Papelera
    assert!(instance::delete(&mgr, &cfg.id, "otro").is_err());
    instance::delete(&mgr, &cfg.id, "Prueba 939").unwrap();
    assert!(!paths::server_dir(&cfg.id).exists());
    println!("✔ borrado seguro a la Papelera");
}

/// Lee los metadatos de una carpeta de mods real (copias) sin internet.
/// HOST939_TEST_MODS=/ruta/a/copia/mods cargo test -p host939-core --test server_lifecycle mods_metadata -- --ignored --nocapture
#[test]
#[ignore]
fn mods_metadata_from_real_copies() {
    let dir = std::env::var("HOST939_TEST_MODS").expect("definí HOST939_TEST_MODS");
    let mut client = vec![];
    let mut total = 0;
    let mut ids = std::collections::HashMap::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let Some(m) = host939_core::mods::jar_meta(&e.path()) else { println!("  sin metadatos: {:?}", e.file_name()); continue };
        total += 1;
        *ids.entry(m.id.clone()).or_insert(0) += 1;
        if m.environment == "client" { client.push(m.name.clone()); }
    }
    client.sort();
    println!("✔ {total} mods leídos; solo cliente (Fabric los ignora en el servidor): {}", client.join(", "));
    assert!(total > 0);
    assert!(ids.values().all(|c| *c == 1), "ids repetidos en la copia");
}

/// Jugadores y permisos sobre un servidor real (modo offline para no depender de cuentas).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn players_and_permissions_real_server() {
    use host939_core::players::{self, PlayerAction};
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    java_override();
    if let Ok(jar) = std::env::var("HOST939_TEST_SERVER_JAR") {
        let sha = String::from_utf8(std::process::Command::new("sha1sum").arg(&jar).output().unwrap().stdout).unwrap().split_whitespace().next().unwrap().to_string();
        std::fs::create_dir_all(paths::cache().join("jars")).unwrap();
        std::fs::copy(&jar, paths::cache().join("jars").join(format!("{sha}.jar"))).unwrap();
    }
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let input = CreateServerInput {
        name: "Permisos".into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Vanilla,
        memory_mb: 1536, difficulty: "easy".into(), gamemode: "survival".into(), max_players: 4, port: 25615,
        online_mode: false, whitelist: true, view_distance: 3, simulation_distance: 3, level_seed: "939".into(),
        accept_eula: true, import_world_path: None, optimize: vec![],
    };
    let cfg = instance::create(input, 8192).await.unwrap();
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed);

    // Apagado: se escriben los archivos con el formato de Minecraft.
    players::apply(&mgr, &cfg, "Steve", PlayerAction::WhitelistAdd, None).await.unwrap();
    players::apply(&mgr, &cfg, "Alex", PlayerAction::Op, None).await.unwrap();
    players::apply(&mgr, &cfg, "Alex", PlayerAction::WhitelistAdd, None).await.unwrap();
    assert!(players::apply(&mgr, &cfg, "x; stop", PlayerAction::Op, None).await.is_err(), "nombres raros se rechazan");
    let wl = std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("whitelist.json")).unwrap();
    assert!(wl.contains(&players::offline_uuid("Steve")), "{wl}");
    println!("✔ apagado: lista blanca y operadores guardados con UUID offline correcto");

    mgr.start(&probe, &cfg).unwrap();
    assert!(wait_for(Duration::from_secs(180), || mgr.runtime(&cfg.id).lock().unwrap().status() == RunStatus::Running));
    mgr.send_command(&cfg.id, "whitelist list").unwrap();
    assert!(wait_for(Duration::from_secs(10), || rec.lines.lock().unwrap().iter().any(|l| l.contains("whitelisted") && l.contains("Steve") && l.contains("Alex"))),
        "Minecraft no leyó la lista blanca: {:?}", rec.lines.lock().unwrap().iter().rev().take(5).collect::<Vec<_>>());
    println!("✔ Minecraft aceptó la lista blanca escrita por 939 Host");

    // Encendido: acciones por comando; el servidor actualiza sus archivos.
    let r = players::apply(&mgr, &cfg, "Troll", PlayerAction::Ban, Some("griefing")).await;
    println!("    ban de alguien que nunca entró: {r:?}");
    let msg = players::apply(&mgr, &cfg, "Steve", PlayerAction::Ban, Some("griefing")).await.unwrap();
    println!("    respuesta: {msg}");
    let banned = paths::server_run_dir(&cfg.id).join("banned-players.json");
    assert!(wait_for(Duration::from_secs(10), || std::fs::read_to_string(&banned).map(|t| t.to_lowercase().contains("steve") && t.contains("griefing")).unwrap_or(false)));
    players::apply(&mgr, &cfg, "Steve", PlayerAction::Pardon, None).await.unwrap();
    players::apply(&mgr, &cfg, "Steve", PlayerAction::WhitelistRemove, None).await.unwrap();
    assert!(wait_for(Duration::from_secs(10), || !std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("whitelist.json")).unwrap().contains("Steve")));
    let list = players::list(&mgr, &cfg);
    let a = list.iter().find(|p| p.name == "Alex").unwrap();
    assert!(a.whitelisted && a.op_level == Some(4));
    assert!(!list.iter().any(|p| p.name.eq_ignore_ascii_case("steve") && p.banned), "perdonado");
    println!("✔ encendido: ban y quitar de la lista blanca por comando; la vista refleja todo");
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(60)));
}

/// Puntos incrementales sobre una COPIA de un mundo real (no el original).
/// HOST939_TEST_WORLD=/ruta/a/copia cargo test -p host939-core --test server_lifecycle snapshots_on_real -- --ignored --nocapture
#[test]
#[ignore]
fn snapshots_on_real_world_copy() {
    let src = std::env::var("HOST939_TEST_WORLD").expect("definí HOST939_TEST_WORLD");
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let mgr = Manager::new(Arc::new(host939_core::events::NoEvents));
    let id = "mundo-real";
    let run = paths::server_run_dir(id);
    std::fs::create_dir_all(&run).unwrap();
    let st = std::process::Command::new("cp").arg("-r").arg(&src).arg(run.join("world")).status().unwrap();
    assert!(st.success());
    let size = paths::dir_size(&run.join("world"));
    let t = Instant::now();
    let s1 = snapshots::take(&mgr, id, "inicial", "manual", "26.2").unwrap();
    let t1 = t.elapsed();
    let store1 = paths::dir_size(&paths::server_dir(id).join("restore"));
    println!("✔ primer punto: {} archivos, {:.1} MB de mundo → {:.1} MB guardados, {:.2} s", s1.file_count, size as f64 / 1e6, store1 as f64 / 1e6, t1.as_secs_f64());

    // "Se juega": cambian algunos bloques de 64 KiB de dos regiones y el level.dat.
    use std::io::{Seek, SeekFrom, Write};
    for f in ["dimensions/minecraft/overworld/region/r.-1.2.mca", "dimensions/minecraft/overworld/region/r.-2.2.mca"] {
        let mut h = std::fs::OpenOptions::new().write(true).open(run.join("world").join(f)).unwrap();
        h.seek(SeekFrom::Start(3 * 65536 + 17)).unwrap();
        h.write_all(b"cambio-939").unwrap();
    }
    std::fs::write(run.join("world/level.dat"), b"level-modificado").unwrap();
    let t = Instant::now();
    let s2 = snapshots::take(&mgr, id, "despues", "auto", "26.2").unwrap();
    let store2 = paths::dir_size(&paths::server_dir(id).join("restore"));
    println!("✔ segundo punto: +{:.2} MB nuevos en el almacén, {:.2} s", (store2 - store1) as f64 / 1e6, t.elapsed().as_secs_f64());
    assert!(store2 - store1 < 1_000_000, "debe guardar solo lo cambiado");

    // Restaurar el primero y comparar con la copia de origen, byte a byte.
    snapshots::restore(&mgr, id, &s1.id, "26.2").unwrap();
    let out = std::process::Command::new("diff").arg("-r").arg(&src).arg(run.join("world")).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    println!("✔ restaurar el primer punto deja el mundo idéntico a la copia original (diff -r sin diferencias)");
    let _ = s2;
}

/// Importa una carpeta de mods real (copia) a un servidor de prueba.
#[test]
#[ignore]
fn import_mods_from_real_copy() {
    let dir = std::env::var("HOST939_TEST_MODS").expect("definí HOST939_TEST_MODS");
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let before: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect();
    let mut cfg: ServerConfig = serde_json::from_value(serde_json::json!({
        "id": "imp", "name": "x", "description": "", "mcVersion": "26.2", "loader": "fabric", "loaderVersion": "0.20.1", "javaMajor": 25,
        "memoryMb": 1024, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565, "maxPlayers": 4, "motd": "", "onlineMode": true,
        "whitelist": false, "difficulty": "easy", "gamemode": "survival", "hardcore": false, "pvp": true, "levelSeed": "", "levelType": "minecraft:normal",
        "viewDistance": 8, "simulationDistance": 6, "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15,
        "restoreIntervalMin": 15, "installed": true, "installError": null, "eulaAcceptedAt": "x", "createdAt": "x",
        "identity": { "publicKey": "a", "accessCode": "X" }, "extraProperties": {}
    })).unwrap();
    cfg.id = "imp".into();
    let r = host939_core::mods::import_folder(&cfg, std::path::Path::new(&dir)).unwrap();
    println!("✔ copiados {}, solo cliente omitidos {}, duplicados {}", r.copied.len(), r.skipped_client.len(), r.skipped_duplicate.len());
    println!("    omitidos por ser de cliente: {}", r.skipped_client.join(", "));
    let r2 = host939_core::mods::import_folder(&cfg, std::path::Path::new(&dir)).unwrap();
    assert!(r2.copied.is_empty(), "una segunda importación no copia nada");
    let after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name(), e.metadata().unwrap().len())).collect();
    assert_eq!(before, after, "la carpeta de origen no se modifica");
    assert!(r.skipped_client.len() >= 5);
}

/// Ensayo de migración con COPIAS de un modpack privado: servidor Fabric 26.2 creado por 939 Host,
/// mundo importado (copia), mods importados (copias), arranque real y punto de restauración.
/// HOST939_TEST_JAVA25=… HOST939_TEST_SERVER_JAR=… HOST939_TEST_WORLD=… HOST939_TEST_MODS=…
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn migration_rehearsal_with_copies() {
    let world = std::env::var("HOST939_TEST_WORLD").expect("HOST939_TEST_WORLD");
    let mods = std::env::var("HOST939_TEST_MODS").expect("HOST939_TEST_MODS");
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    java_override();
    if let Ok(jar) = std::env::var("HOST939_TEST_SERVER_JAR") {
        let sha = String::from_utf8(std::process::Command::new("sha1sum").arg(&jar).output().unwrap().stdout).unwrap().split_whitespace().next().unwrap().to_string();
        std::fs::create_dir_all(paths::cache().join("jars")).unwrap();
        std::fs::copy(&jar, paths::cache().join("jars").join(format!("{sha}.jar"))).unwrap();
    }
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let input = CreateServerInput {
        name: "Modpack prueba".into(), description: "copia".into(), mc_version: "26.2".into(), loader: Loader::Fabric,
        memory_mb: 3072, difficulty: "hard".into(), gamemode: "survival".into(), max_players: 8, port: 25619,
        online_mode: false, whitelist: false, view_distance: 6, simulation_distance: 4, level_seed: String::new(),
        accept_eula: true, import_world_path: Some(world.clone()), optimize: vec![],
    };
    let cfg = instance::create(input, 8192).await.expect("crear");
    println!("✔ creado Fabric {} (loader {:?})", cfg.mc_version, cfg.loader_version);
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "instalación: {:?} / {:?}", cfg.install_error, rec.progress.lock().unwrap());
    println!("✔ instalado: {:?}", host939_core::process::launch_target(&cfg));
    let r = host939_core::mods::import_folder(&cfg, std::path::Path::new(&mods)).unwrap();
    println!("✔ mods importados: {} (omitidos solo cliente: {})", r.copied.len(), r.skipped_client.len());
    let t = Instant::now();
    mgr.start(&probe, &cfg).unwrap();
    let ok = wait_for(Duration::from_secs(400), || matches!(mgr.runtime(&cfg.id).lock().unwrap().status(), RunStatus::Running | RunStatus::Crashed | RunStatus::Stopped));
    let status = mgr.runtime(&cfg.id).lock().unwrap().status();
    assert!(ok && status == RunStatus::Running, "estado {status:?}: {:?}", rec.lines.lock().unwrap().iter().rev().take(15).collect::<Vec<_>>());
    println!("✔ modpack (copia) en línea en {:.0} s con {} mods", t.elapsed().as_secs_f32(), r.copied.len());
    let mem = mgr.runtime(&cfg.id).lock().unwrap().pid.and_then(|p| host939_core::hardware::process_memory_mb(&probe, p));
    println!("    memoria del proceso: {mem:?} MB");
    std::thread::sleep(Duration::from_secs(25));
    let mspt = mgr.runtime(&cfg.id).lock().unwrap().mspt;
    println!("    MSPT medido: {mspt:?}");
    let s = snapshots::take(&mgr, &cfg.id, "migración", "manual", "26.2").unwrap();
    println!("✔ punto de restauración en caliente: {} archivos, pausa de guardado {} ms", s.file_count, s.pause_ms);
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
    let errors: Vec<String> = rec.lines.lock().unwrap().iter().filter(|l| l.contains("/ERROR]") && !l.contains("yggdrasil") && !l.contains("minecraftservices") && !l.contains("UnknownHost")).cloned().collect();
    println!("    errores en el registro: {}", errors.len());
    for e in errors.iter().take(5) { println!("      {e}"); }
    println!("✔ detenido limpio");
}
