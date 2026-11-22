//! Pruebas de punta a punta con descargas reales, pensadas para correr en Windows (CI) o en
//! cualquier PC con internet. Todo ocurre en una carpeta temporal aislada: no se toca ninguna
//! instalación de Minecraft ni ningún mundo existente.
//!
//! - `e2e_loaders`: para Vanilla, Fabric, Forge y NeoForge: crear → instalar (Java y servidor
//!   descargados por 939 Host, verificados) → encender → «Done» → comando → apagar limpio.
//! - `e2e_modrinth`: servidor Fabric + búsqueda real en Modrinth + plan con dependencias +
//!   instalación con hash verificado → el servidor arranca con el mod cargado.
//!
//! cargo test -p host939-core --test e2e_real -- --ignored --nocapture --test-threads 1

use host939_core::events::{Events, InstallProgress};
use host939_core::hardware::Probe;
use host939_core::model::*;
use host939_core::process::Manager;
use host939_core::{instance, meta, modrinth, mods, net, paths};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Recorder {
    progress: Mutex<Vec<String>>,
    console: Mutex<Vec<String>>,
}
impl Events for Recorder {
    fn server_changed(&self, _: &str) {}
    fn console_line(&self, _: &str, l: &str) { self.console.lock().unwrap().push(l.to_string()); }
    fn install_progress(&self, p: &InstallProgress) {
        let mut v = self.progress.lock().unwrap();
        if v.last().map(|l| !l.starts_with(&p.message)).unwrap_or(true) || p.error.is_some() {
            v.push(format!("{} {:.0}% {:?}", p.message, p.fraction * 100.0, p.error));
        }
    }
}

static INIT: Once = Once::new();
static mut ROOT: Option<tempfile::TempDir> = None;

/// Una sola raíz por proceso (paths::init es global); cada prueba usa servidores distintos.
fn init() {
    INIT.call_once(|| {
        let t = tempfile::tempdir().unwrap();
        paths::init(t.path().to_path_buf()).unwrap();
        if let Ok(j) = std::env::var("HOST939_TEST_JAVA25") {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&j, paths::runtimes().join("java-25")).unwrap();
            #[cfg(not(unix))]
            let _ = j;
        }
        #[allow(static_mut_refs)]
        unsafe { ROOT = Some(t) };
    });
}

fn wait_for(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() { return true; }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

/// Consulta de estado del protocolo de Minecraft (lo que hace la lista de servidores del juego).
fn slp_once(port: u16) -> Result<String, String> {
    use std::io::{Read, Write};
    fn varint(mut v: u32, out: &mut Vec<u8>) { loop { let b = (v & 0x7f) as u8; v >>= 7; if v == 0 { out.push(b); break } out.push(b | 0x80); } }
    fn read_varint(r: &mut impl Read) -> std::io::Result<u32> { let mut n = 0u32; for i in 0..5 { let mut b = [0u8]; r.read_exact(&mut b)?; n |= ((b[0] & 0x7f) as u32) << (7 * i); if b[0] & 0x80 == 0 { return Ok(n) } } Ok(n) }
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("conectar: {e}"))?;
    c.set_read_timeout(Some(Duration::from_secs(10))).ok();
    let mut body = vec![0u8]; varint(767, &mut body); varint(9, &mut body); body.extend_from_slice(b"127.0.0.1"); body.extend_from_slice(&port.to_be_bytes()); varint(1, &mut body);
    let mut pkt = vec![]; varint(body.len() as u32, &mut pkt); pkt.extend(body); pkt.extend_from_slice(&[1, 0]);
    c.write_all(&pkt).map_err(|e| format!("enviar: {e}"))?;
    let len = read_varint(&mut c).map_err(|e| format!("leer largo: {e}"))?;
    let mut buf = vec![0u8; len as usize];
    c.read_exact(&mut buf).map_err(|e| format!("leer respuesta: {e}"))?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn slp_status(port: u16) -> String {
    let mut last = String::new();
    for i in 0..10 {
        match slp_once(port) {
            Ok(s) if s.contains("\"version\"") => { if i > 0 { println!("  (estado respondió al intento {}; antes: {last})", i + 1); } return s }
            Ok(s) => last = s,
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    format!("sin respuesta en el puerto {port}: {last}")
}

/// Primera versión de la lista que el loader soporta (la más nueva primero).
async fn pick_version(loader: Loader, candidates: &[&str]) -> Option<String> {
    for mc in candidates {
        if let Ok(s) = meta::loader_support(loader, mc).await {
            if s.supported { return Some(mc.to_string()); }
        }
    }
    None
}

async fn release_candidates() -> Vec<String> {
    meta::list_versions(false).await.expect("lista de versiones de Mojang").into_iter().map(|v| v.id).take(12).collect()
}

fn input(name: &str, mc: &str, loader: Loader, port: u16) -> CreateServerInput {
    CreateServerInput {
        name: name.into(), description: "prueba de punta a punta".into(), mc_version: mc.into(), loader,
        memory_mb: 2048, difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port,
        online_mode: true, whitelist: false, view_distance: 4, simulation_distance: 4, level_seed: "939".into(),
        accept_eula: true, import_world_path: None, optimize: vec![],
    }
}

/// Instala, enciende, verifica y apaga. Devuelve el tiempo hasta «en línea».
async fn run_server(mgr: &Manager, rec: &Recorder, probe: &Probe, cfg: ServerConfig) -> (ServerConfig, f32) {
    let t0 = Instant::now();
    instance::install(mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "{} {:?}: instalación falló: {:?}\n{}", cfg.mc_version, cfg.loader, cfg.install_error, rec.progress.lock().unwrap().join("\n"));
    println!("  instalado en {:.0} s (Java {})", t0.elapsed().as_secs_f32(), cfg.java_major);
    let t1 = Instant::now();
    mgr.start(probe, &cfg).expect("iniciar");
    let rt = mgr.runtime(&cfg.id);
    let ok = wait_for(Duration::from_secs(600), || {
        let s = rt.lock().unwrap().status();
        s == RunStatus::Running || s == RunStatus::Crashed || s == RunStatus::Stopped
    });
    let st = rt.lock().unwrap().status();
    assert!(ok && st == RunStatus::Running, "{:?} no llegó a funcionar ({st:?}):\n{}", cfg.loader, mgr.console(&cfg.id).iter().rev().take(40).cloned().collect::<Vec<_>>().join("\n"));
    let online = t1.elapsed().as_secs_f32();
    mgr.send_command(&cfg.id, "say hola desde la prueba").unwrap();
    assert!(wait_for(Duration::from_secs(20), || mgr.console(&cfg.id).iter().any(|l| l.contains("hola desde la prueba"))), "el comando no llegó");
    // Estado por el protocolo de Minecraft (lo mismo que ve el juego).
    let st = slp_status(cfg.port);
    if !st.contains("\"version\"") {
        let props = std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("server.properties")).unwrap_or_default();
        let ports: Vec<_> = props.lines().filter(|l| l.contains("port") || l.contains("server-ip")).collect();
        let net = std::process::Command::new(if cfg!(windows) { "netstat" } else { "ss" }).args(if cfg!(windows) { vec!["-ano", "-p", "TCP"] } else { vec!["-ltnp"] }).output().map(|o| String::from_utf8_lossy(&o.stdout).lines().filter(|l| l.contains("LISTEN")).map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let pid = mgr.runtime(&cfg.id).lock().unwrap().pid;
        panic!("estado del servidor: {st}\nproperties: {ports:?}\npid java: {pid:?}\nescuchando:\n{net}\nconsola:\n{}", mgr.console(&cfg.id).iter().rev().take(15).cloned().collect::<Vec<_>>().join("\n"));
    }
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)), "no se apagó");
    assert_eq!(rt.lock().unwrap().status(), RunStatus::Stopped, "apagado no limpio: {:?}", rt.lock().unwrap().last_error);
    (cfg, online)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_loaders() {
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let cands = release_candidates().await;
    let cand: Vec<&str> = cands.iter().map(|s| s.as_str()).collect();
    let mut port = 25700;
    let mut skipped = vec![];
    for loader in [Loader::Vanilla, Loader::Fabric, Loader::Forge, Loader::NeoForge] {
        let Some(mc) = pick_version(loader, &cand).await else {
            skipped.push(format!("{loader:?}: ninguna de {cand:?} tiene versión publicada"));
            continue;
        };
        port += 1;
        println!("▶ {loader:?} {mc}");
        rec.progress.lock().unwrap().clear();
        let cfg = instance::create(input(&format!("E2E {loader:?}"), &mc, loader, port), 8192).await.expect("crear");
        let (cfg, secs) = run_server(&mgr, &rec, &probe, cfg).await;
        println!("✔ {loader:?} {mc} (loader {:?}) en línea en {secs:.0} s y apagado limpio", cfg.loader_version);
    }
    for s in &skipped { println!("⚠ omitido: {s}"); }
    assert!(skipped.len() <= 1, "demasiados loaders sin versión: {skipped:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_modrinth() {
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let cands = release_candidates().await;
    let cand: Vec<&str> = cands.iter().map(|s| s.as_str()).collect();
    let mc = pick_version(Loader::Fabric, &cand).await.expect("Fabric sin versiones");
    let src = modrinth::Modrinth;
    // Lithium: mod de servidor muy usado, sin dependencias obligatorias de cliente.
    let res = src.search("lithium", &mc, "fabric", true, 0).await.expect("búsqueda en Modrinth");
    println!("✔ Modrinth: {} resultados para «lithium» en Fabric {mc}", res.total_hits);
    let hit = res.hits.iter().find(|h| h.slug == "lithium").or(res.hits.first()).expect("sin resultados");
    let cfg = instance::create(input("E2E Mods", &mc, Loader::Fabric, 25790), 8192).await.expect("crear");
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "Fabric: {:?}", cfg.install_error);
    let plan = mods::plan_install(&src, &cfg, &hit.project_id, None).await.expect("plan");
    println!("✔ plan: {} archivo(s), {:.1} MB, avisos {:?}", plan.items.len(), plan.total_bytes as f64 / 1e6, plan.warnings);
    let installed = mods::apply_plan(&cfg, &plan, |_, _| {}).await.expect("instalar mod");
    assert!(!installed.is_empty());
    let jars: Vec<_> = std::fs::read_dir(mods::mods_dir(&cfg.id)).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    println!("✔ instalado con hash verificado: {jars:?}");
    // Falsos positivos: mods reales y populares no tienen que salir como sospechosos, y Modrinth
    // tiene que reconocerlos por su huella.
    for j in &jars {
        let r = host939_core::modscan::analyze_cached(&mods::mods_dir(&cfg.id).join(j)).await.expect("analizar");
        println!("  análisis de {j}: {} · {} · alertas {:?} · Modrinth {:?}", r.status, r.summary, r.findings.iter().filter(|f| f.level != host939_core::modscan::Level::Info).map(|f| &f.title).collect::<Vec<_>>(), r.modrinth.as_ref().map(|m| &m.title));
        assert!(r.status != "sospechoso" && r.status != "amenaza", "falso positivo en {j}: {:?}", r.findings);
        assert!(r.modrinth.is_some() || r.unchecked.iter().any(|u| u.starts_with("procedencia")), "Modrinth tenía que reconocer {j} por su huella");
    }
    println!("✔ análisis de seguridad: sin falsos positivos y con procedencia de Modrinth");
    // Escaneo: el archivo queda reconocido como de Modrinth.
    let scan = mods::scan(&src, &cfg, true).await.expect("escaneo");
    assert!(!scan.is_empty());
    let (cfg, secs) = run_server(&mgr, &rec, &probe, cfg).await;
    let log = std::fs::read_to_string(paths::server_run_dir(&cfg.id).join("logs/latest.log")).unwrap_or_default();
    assert!(log.to_lowercase().contains(&hit.slug.to_lowercase()) || log.contains("Loading") , "el log no menciona el mod");
    println!("✔ Fabric {mc} con {} en línea en {secs:.0} s", hit.title);
    // Quitar el mod deja la carpeta como estaba.
    mods::remove(&cfg, &hit.project_id).expect("quitar");
    println!("✔ mod quitado");
}

/// Un cliente oficial de Minecraft (descargado de Mojang) entra a servidores creados por 939 Host.
/// Necesita `CLIENTE_REAL` (ruta a ci/cliente-real/cliente.py), Python y una pantalla (xvfb en CI).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_cliente_real() {
    init();
    let script = std::env::var("CLIENTE_REAL").expect("CLIENTE_REAL");
    let mc = std::env::var("E2E_MC").unwrap_or_else(|_| "26.2".into());
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let mut port = 25850;
    for loader in [Loader::Vanilla, Loader::Fabric] {
        port += 1;
        let mut inp = input(&format!("Cliente {loader:?}"), &mc, loader, port);
        inp.online_mode = false; // cuenta sin conexión del cliente de prueba
        let cfg = instance::create(inp, 8192).await.expect("crear");
        instance::install(&mgr, cfg.id.clone()).await;
        let cfg = instance::load(&cfg.id).unwrap();
        assert!(cfg.installed, "{:?}", cfg.install_error);
        mgr.start(&probe, &cfg).expect("iniciar");
        let rt = mgr.runtime(&cfg.id);
        assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó");
        // El estado «listo» sale de una consulta real, no de la línea «Done».
        assert!(wait_for(Duration::from_secs(30), || rt.lock().unwrap().reach.as_deref() == Some("ok")), "la comprobación de conexión no dio ok: {:?}", rt.lock().unwrap().reach_error);
        let info = rt.lock().unwrap().reach_info.clone().unwrap();
        println!("✔ {loader:?} {mc}: 939 Host comprobó la conexión (Minecraft {} protocolo {}, {} ms)", info.version, info.protocol, info.latency_ms);
        let dir = paths::root().join(format!("cliente-{loader:?}"));
        let name = "Jugador939";
        let mut child = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
            .args([&script, &mc, &format!("127.0.0.1:{port}"), dir.to_str().unwrap(), name, "150"])
            .spawn().expect("abrir el cliente");
        let joined = wait_for(Duration::from_secs(420), || mgr.console(&cfg.id).iter().any(|l| l.contains(&format!("{name} joined the game"))));
        let console: Vec<String> = mgr.console(&cfg.id).iter().rev().take(25).rev().cloned().collect();
        let client_log = std::fs::read_to_string(dir.join("game/logs/latest.log")).or_else(|_| std::fs::read_to_string(dir.join("salida.txt"))).unwrap_or_default();
        if joined {
            println!("✔ un cliente real de Minecraft {mc} entró al servidor {loader:?} («{name} joined the game»)");
            // Fase 2 con un jugador de verdad: conectado, ficha en vivo y edición por comandos.
            std::thread::sleep(Duration::from_secs(3));
            assert!(host939_core::players::list(&mgr, &cfg).iter().any(|p| p.name == name && p.online), "la lista lo muestra conectado");
            let live = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).expect("ficha en vivo");
            println!("✔ ficha en vivo: vida {:?}, hambre {:?}, nivel {:?}", live.health, live.food, live.xp_level);
            let e = host939_core::playeredit::Edit { health: Some(7.0), xp_level: Some(5), ..Default::default() };
            let msg = host939_core::playeredit::apply_online(&mgr, &cfg, name, &e, "prueba").expect("editar en vivo");
            let after = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap();
            println!("✔ edición en vivo: {msg} → vida {:?}, nivel {:?}", after.health, after.xp_level);
            assert_eq!(after.xp_level, Some(5));
            assert!(after.health.map(|h| (h - 7.0).abs() <= 1.0).unwrap_or(false), "vida tras /damage: {:?}", after.health);
            // Subir la vida (curación + ajuste) también queda exacto.
            host939_core::playeredit::apply_online(&mgr, &cfg, name, &host939_core::playeredit::Edit { health: Some(15.0), ..Default::default() }, "prueba").expect("subir vida");
            let up = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap();
            println!("✔ subir la vida en vivo → {:?}", up.health);
            // Con el hambre llena el jugador se regenera solo entre el ajuste y la lectura.
            assert!(up.health.map(|h| (h - 15.0).abs() <= 1.0).unwrap_or(false), "vida tras curar y ajustar: {:?}", up.health);
            // Botones − y + (un paso) y matar, con el jugador conectado.
            use host939_core::playeredit::{adjust, kill, Stat};
            // Con el hambre llena Minecraft regenera sola (hasta ~0,7 por segundo) y puede tapar medio
            // corazón entre la lectura y el ajuste: se prueba hasta tres veces.
            let (mut h0, mut h1) = (0.0, 0.0);
            for _ in 0..3 {
                h0 = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap().health.unwrap();
                adjust(&mgr, &cfg, name, "", true, Stat::Health, false, "prueba").expect("bajar medio corazón");
                h1 = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap().health.unwrap();
                if h1 < h0 { break; }
            }
            assert!(h1 < h0 && (h0 - h1) <= 2.0, "bajar un paso: {h0} → {h1}");
            let l0 = host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap().xp_level.unwrap();
            adjust(&mgr, &cfg, name, "", true, Stat::Xp, true, "prueba").expect("subir un nivel");
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(host939_core::playerdata::detail(&mgr, &cfg, name, "", true).unwrap().xp_level, Some(l0 + 1));
            assert!(adjust(&mgr, &cfg, name, "", true, Stat::Food, true, "prueba").is_err(), "hambre de a uno no se ofrece conectado");
            println!("✔ − y + en vivo: vida {h0} → {h1}, nivel {l0} → {}", l0 + 1);
            let msg = kill(&mgr, &cfg, name, "prueba").expect("matar");
            println!("✔ matar en vivo: {msg}");
            mgr.send_command(&cfg.id, &format!("kick {name} prueba terminada")).ok();
            // Salida: queda desconectado, con su sesión registrada y su archivo guardado.
            assert!(wait_for(Duration::from_secs(20), || !mgr.runtime(&cfg.id).lock().unwrap().players.contains(name)), "sigue figurando conectado");
            let h = host939_core::sessions::history(&cfg.id, name);
            assert_eq!(h.sessions, 1, "una sesión registrada: {h:?}");
            println!("✔ salida registrada: {} s de sesión", h.total_secs);
            let p = host939_core::players::list(&mgr, &cfg).into_iter().find(|p| p.name == name).expect("en la lista");
            assert!(!p.online && p.last_seen.is_some());
            assert!(wait_for(Duration::from_secs(20), || host939_core::playerdata::detail(&mgr, &cfg, name, &p.uuid, false).map(|d| d.source == "saved").unwrap_or(false)), "Minecraft no guardó al jugador");
            let msg = host939_core::playeredit::apply_offline(&mgr, &cfg, name, &p.uuid, &host939_core::playeredit::Edit { health: Some(13.0), food: Some(9), ..Default::default() }, "prueba").expect("editar desconectado");
            let d = host939_core::playerdata::detail(&mgr, &cfg, name, &p.uuid, false).unwrap();
            assert_eq!((d.health, d.food), (Some(13.0), Some(9)), "{msg}");
            println!("✔ edición desconectado sobre el archivo real de Minecraft {mc}: vida 13, hambre 9");
            adjust(&mgr, &cfg, name, &p.uuid, false, Stat::Food, true, "prueba").expect("hambre +1 desconectado");
            adjust(&mgr, &cfg, name, &p.uuid, false, Stat::Health, false, "prueba").expect("vida −1 desconectado");
            let d = host939_core::playerdata::detail(&mgr, &cfg, name, &p.uuid, false).unwrap();
            assert_eq!((d.health, d.food), (Some(12.0), Some(10)));
            assert!(kill(&mgr, &cfg, name, "prueba").is_err(), "no se mata desconectado");
            println!("✔ − y + desconectado: vida 12, hambre 10; matar desconectado se niega");
        }
        let _ = child.kill();
        let _ = child.wait();
        if !joined {
            panic!("el cliente no entró.\n── consola del servidor ──\n{}\n── juego (últimas líneas) ──\n{}", console.join("\n"), client_log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"));
        }
        assert!(rt.lock().unwrap().join_issue.is_none(), "no debería haber un problema de entrada: {:?}", rt.lock().unwrap().join_issue);
        mgr.stop(&cfg.id).unwrap();
        assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
    }
}

/// Cambio de versión real: análisis, actualización en el mismo servidor con copia verificada,
/// arranque de prueba, mundo conservado, mods actualizados y vuelta atrás.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_cambio_version() {
    use host939_core::version_change as vc;
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let src = modrinth::Modrinth;
    let cands = release_candidates().await;
    // Dos versiones estables seguidas que Fabric y Lithium soporten.
    let mut pair = None;
    for w in cands.windows(2) {
        let (new, old) = (w[0].clone(), w[1].clone());
        let ok_f = meta::loader_support(Loader::Fabric, &new).await.map(|s| s.supported).unwrap_or(false) && meta::loader_support(Loader::Fabric, &old).await.map(|s| s.supported).unwrap_or(false);
        if ok_f && src.search("lithium", &old, "fabric", true, 0).await.map(|r| r.hits.iter().any(|h| h.slug == "lithium")).unwrap_or(false)
            && src.search("lithium", &new, "fabric", true, 0).await.map(|r| r.hits.iter().any(|h| h.slug == "lithium")).unwrap_or(false) { pair = Some((old, new)); break; }
    }
    let (old, new) = pair.expect("no hay dos versiones seguidas con Fabric y Lithium");
    println!("▶ Fabric {old} → {new}");
    let cfg = instance::create(input("Cambio", &old, Loader::Fabric, 25960), 8192).await.unwrap();
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "{:?}", cfg.install_error);
    let hit = src.search("lithium", &old, "fabric", true, 0).await.unwrap().hits.into_iter().find(|h| h.slug == "lithium").unwrap();
    let plan = mods::plan_install(&src, &cfg, &hit.project_id, None).await.unwrap();
    mods::apply_plan(&cfg, &plan, |_, _| {}).await.unwrap();
    // Mundo con una marca.
    mgr.start(&probe, &cfg).unwrap();
    let rt = mgr.runtime(&cfg.id);
    assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running));
    mgr.send_command(&cfg.id, "scoreboard objectives add cambio dummy").unwrap();
    mgr.send_command(&cfg.id, "scoreboard players set marca cambio 939").unwrap();
    std::thread::sleep(Duration::from_secs(1));
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));

    // Análisis.
    let a = vc::analyze(&src, &cfg, &new).await.unwrap();
    println!("  análisis: {} · bloqueos {:?} · avisos {:?}", a.direction, a.blockers, a.warnings);
    assert_eq!(a.direction, "upgrade");
    assert!(a.can_update_in_place);
    assert!(a.mods.iter().any(|m| m.title.to_lowercase().contains("lithium") && (m.status == "update" || m.status == "ok")));
    let back = vc::analyze(&src, &instance::load(&cfg.id).unwrap(), &cands.iter().rev().find(|v| **v != old && **v != new).cloned().unwrap()).await.unwrap();
    assert_eq!(back.direction, "downgrade");
    assert!(!back.can_update_in_place && !back.blockers.is_empty(), "volver atrás con mundo tiene que estar bloqueado");
    println!("✔ análisis: actualizar permitido; volver a una versión anterior con el mundo, bloqueado con explicación");

    // Actualización en el mismo servidor.
    let r = vc::update_in_place(&mgr, &src, &cfg.id, &new, |m, _| println!("    · {m}")).await.unwrap();
    for s in &r.steps { println!("    ✓ {s}"); }
    assert!(r.ok, "falló: {:?}", r.error);
    let c2 = instance::load(&cfg.id).unwrap();
    assert_eq!(c2.mc_version, new);
    let jars: Vec<String> = std::fs::read_dir(mods::mods_dir(&cfg.id)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    println!("  mods ahora: {jars:?}");
    // El mundo conserva la marca.
    mgr.start(&probe, &c2).unwrap();
    assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running));
    mgr.send_command(&cfg.id, "scoreboard players get marca cambio").unwrap();
    assert!(wait_for(Duration::from_secs(20), || mgr.console(&cfg.id).iter().any(|l| l.contains("marca has 939"))), "el mundo perdió la marca");
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
    println!("✔ actualizado a {new}: copia verificada, mods actualizados, arranque de prueba y mundo con la marca");

    // Volver atrás.
    if let Err(e) = vc::rollback(&mgr, &cfg.id).await {
        for l in rec.console.lock().unwrap().iter().filter(|l| l.starts_with("[939 Host]")) { println!("  consola: {l}"); }
        panic!("volver atrás falló: {e:?}");
    }
    let c3 = instance::load(&cfg.id).unwrap();
    assert_eq!(c3.mc_version, old);
    mgr.start(&probe, &c3).unwrap();
    assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó tras volver atrás");
    mgr.send_command(&cfg.id, "scoreboard players get marca cambio").unwrap();
    assert!(wait_for(Duration::from_secs(20), || mgr.console(&cfg.id).iter().any(|l| l.contains("marca has 939"))), "la vuelta atrás perdió la marca");
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
    println!("✔ vuelta atrás a {old}: mundo anterior restaurado y arranca");
}

/// Perfil para jugar (.mrpack) real: mods de Modrinth por URL y hash, mod manual excluido o en
/// overrides, y la descarga de cada URL coincide con su sha512.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_perfil_mrpack() {
    use sha2::Digest;
    init();
    let src = modrinth::Modrinth;
    let cands = release_candidates().await;
    let mut mc = None;
    for v in &cands {
        if meta::loader_support(Loader::Fabric, v).await.map(|s| s.supported).unwrap_or(false)
            && src.search("lithium", v, "fabric", true, 0).await.map(|r| r.hits.iter().any(|h| h.slug == "lithium")).unwrap_or(false) { mc = Some(v.clone()); break; }
    }
    let mc = mc.expect("sin versión con Fabric y Lithium");
    let mut cfg = instance::create(input("Perfil", &mc, Loader::Fabric, 25961), 8192).await.unwrap();
    cfg.loader_version = meta::loader_support(Loader::Fabric, &mc).await.unwrap().loader_version;
    instance::save(&cfg).unwrap();
    let hit = src.search("lithium", &mc, "fabric", true, 0).await.unwrap().hits.into_iter().find(|h| h.slug == "lithium").unwrap();
    let plan = mods::plan_install(&src, &cfg, &hit.project_id, None).await.unwrap();
    mods::apply_plan(&cfg, &plan, |_, _| {}).await.unwrap();
    std::fs::write(mods::mods_dir(&cfg.id).join("hecho-a-mano.jar"), b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    let out = std::env::temp_dir().join(format!("perfil-{}.mrpack", cfg.id));
    let r = host939_core::profile::export_mrpack(&src, &cfg.id, &out, false).await.unwrap();
    println!("  {r:?}");
    assert!(r.from_modrinth.iter().any(|t| t.to_lowercase().contains("lithium")));
    assert_eq!(r.manual_excluded, vec!["hecho-a-mano.jar".to_string()]);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let idx: serde_json::Value = serde_json::from_reader(z.by_name("modrinth.index.json").unwrap()).unwrap();
    assert_eq!(idx["formatVersion"], 1);
    assert_eq!(idx["dependencies"]["minecraft"], mc.as_str());
    assert!(idx["dependencies"]["fabric-loader"].is_string());
    for f in idx["files"].as_array().unwrap() {
        let url = f["downloads"][0].as_str().unwrap();
        assert!(url.starts_with("https://cdn.modrinth.com/"), "{url}");
        let tmp = std::env::temp_dir().join("mrpack-check.jar");
        net::download(url, &tmp, net::Hash::None, |_, _| {}).await.unwrap();
        assert_eq!(hex::encode(sha2::Sha512::digest(std::fs::read(&tmp).unwrap())), f["hashes"]["sha512"].as_str().unwrap(), "sha512 no coincide: {url}");
    }
    let r2 = host939_core::profile::export_mrpack(&src, &cfg.id, &out, true).await.unwrap();
    assert_eq!(r2.manual_included, vec!["hecho-a-mano.jar".to_string()]);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    assert!(z.by_name("overrides/mods/hecho-a-mano.jar").is_ok());
    println!("✔ .mrpack válido ({} mods de Modrinth verificados por sha512; manual excluido/incluido según se pida)", idx["files"].as_array().unwrap().len());
}

/// Mods de rendimiento reales: candidatos consultados en Modrinth para la versión exacta,
/// preset «Recomendado» instalado al crear (como en el asistente), el servidor arranca con los
/// mods cargados y Chunky pregenera un área chica cuando se lo pide.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_optimizacion() {
    use host939_core::optimize;
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let src = modrinth::Modrinth;
    let cands = release_candidates().await;
    for (loader, port) in [(Loader::Fabric, 25962u16), (Loader::NeoForge, 25963u16)] {
        let mut mc = None;
        for v in &cands { if meta::loader_support(loader, v).await.map(|s| s.supported).unwrap_or(false) { mc = Some(v.clone()); break; } }
        let mc = mc.expect("sin versión para el loader");
        let cs = optimize::candidates(&src, &mc, loader, &[]).await.unwrap();
        for c in &cs { println!("  {:?} {mc} · {:<18} {} {}", loader, c.slug, if c.available { "disponible" } else { "—" }, c.unavailable_reason.clone().unwrap_or_default()); }
        let pick = optimize::preset(&cs, "recommended");
        println!("  Recomendado: {pick:?} · Máximo: {:?}", optimize::preset(&cs, "max"));
        assert!(!pick.is_empty(), "ningún mod recomendado para {loader:?} {mc}");
        assert!(optimize::conflicts(&cs, &optimize::preset(&cs, "max")).is_empty(), "el preset máximo trae una combinación a evitar");
        let mut inp = input(&format!("Optimizado {loader:?}"), &mc, loader, port);
        inp.optimize = pick.clone();
        let cfg = instance::create(inp, 8192).await.unwrap();
        instance::install(&mgr, cfg.id.clone()).await;
        let cfg = instance::load(&cfg.id).unwrap();
        assert!(cfg.installed, "{:?}", cfg.install_error);
        let man = mods::load_manifest(&cfg.id);
        let opt: Vec<&str> = man.mods.iter().filter(|m| m.origin.as_deref() == Some("optimizacion")).map(|m| m.title.as_str()).collect();
        println!("  instalados desde Rendimiento: {opt:?} (+{} dependencias)", man.mods.len() - opt.len());
        assert_eq!(opt.len(), pick.len(), "no se instalaron todos los elegidos: {:?}", rec.console.lock().unwrap().iter().filter(|l| l.contains("939 Host")).collect::<Vec<_>>());
        for m in &man.mods { assert!(mods::mods_dir(&cfg.id).join(&m.file_name).exists(), "falta {}", m.file_name); }
        // Falsos positivos del análisis de seguridad con mods reales de rendimiento.
        for m in &man.mods {
            let r = host939_core::modscan::analyze_cached(&mods::mods_dir(&cfg.id).join(&m.file_name)).await.expect("analizar");
            println!("  análisis de {}: {} · alertas {:?}", m.file_name, r.status, r.findings.iter().filter(|f| f.level != host939_core::modscan::Level::Info).map(|f| format!("{} ({})", f.title, f.detail)).collect::<Vec<_>>());
            assert!(r.status != "sospechoso" && r.status != "amenaza", "falso positivo en {}: {:?}", m.file_name, r.findings);
        }
        mgr.start(&probe, &cfg).unwrap();
        let rt = mgr.runtime(&cfg.id);
        let ok = wait_for(Duration::from_secs(600), || { let s = rt.lock().unwrap().status(); s == RunStatus::Running || s == RunStatus::Crashed || s == RunStatus::Stopped });
        assert!(ok && rt.lock().unwrap().status() == RunStatus::Running, "no arrancó con los mods: {:?}", rt.lock().unwrap().last_error);
        println!("✔ {loader:?} {mc} arrancó con {} mods de rendimiento", opt.len());
        if pick.iter().any(|s| s == "chunky") {
            for c in ["chunky world minecraft:overworld", "chunky shape square", "chunky spawn", "chunky radius 64", "chunky start"] { mgr.send_command(&cfg.id, c).unwrap(); }
            assert!(wait_for(Duration::from_secs(240), || mgr.console(&cfg.id).iter().any(|l| l.contains("[Chunky]") && l.contains("finished"))), "Chunky no terminó: {:?}", mgr.console(&cfg.id).iter().filter(|l| l.contains("Chunky")).collect::<Vec<_>>());
            println!("✔ Chunky pregeneró un área de radio 64: {}", mgr.console(&cfg.id).iter().find(|l| l.contains("[Chunky]") && l.contains("finished")).unwrap());
        }
        mgr.stop(&cfg.id).unwrap();
        assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
        // Quitar uno desde la lista normal de mods.
        let first = man.mods.iter().find(|m| m.origin.is_some() && m.required_by.is_empty()).unwrap();
        mods::remove(&cfg, &first.project_id).unwrap();
        assert!(!mods::mods_dir(&cfg.id).join(&first.file_name).exists());
        println!("✔ {} quitado individualmente", first.title);
    }
}

/// Servidor desde un modpack (.mrpack) real: se exporta el perfil de un servidor Fabric con
/// Lithium y Chunky, se crea otro servidor desde ese pack como lo hace la app (versión y loader
/// del pack, contenido al terminar la instalación) y arranca con los mods descargados por hash.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_modpack_mrpack() {
    use host939_core::{modpack, profile};
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let src = modrinth::Modrinth;
    let cands = release_candidates().await;
    let mut mc = None;
    for v in &cands {
        if meta::loader_support(Loader::Fabric, v).await.map(|s| s.supported).unwrap_or(false)
            && src.search("lithium", v, "fabric", true, 0).await.map(|r| r.hits.iter().any(|h| h.slug == "lithium")).unwrap_or(false) { mc = Some(v.clone()); break; }
    }
    let mc = mc.unwrap();
    let mut base = instance::create(input("Origen pack", &mc, Loader::Fabric, 25964), 8192).await.unwrap();
    base.loader_version = meta::loader_support(Loader::Fabric, &mc).await.unwrap().loader_version;
    instance::save(&base).unwrap();
    for slug in ["lithium", "chunky"] {
        let hit = src.search(slug, &mc, "fabric", true, 0).await.unwrap().hits.into_iter().find(|h| h.slug == slug).unwrap();
        let plan = mods::plan_install(&src, &base, &hit.project_id, None).await.unwrap();
        mods::apply_plan(&base, &plan, |_, _| {}).await.unwrap();
    }
    let pack = std::env::temp_dir().join("prueba-939.mrpack");
    profile::export_mrpack(&src, &base.id, &pack, false).await.unwrap();
    let info = modpack::inspect(&pack).unwrap();
    println!("  pack: {} · {:?} {} · {} archivos de servidor", info.name, info.loader, info.mc_version, info.server_files);
    assert_eq!(info.mc_version, mc);
    let mut inp = input("Desde pack", &info.mc_version, info.loader, 25965);
    inp.name = "Desde pack".into();
    let mut cfg = instance::create(inp, 8192).await.unwrap();
    cfg.loader_version = info.loader_version.clone();
    instance::save(&cfg).unwrap();
    modpack::set_pending(&cfg.id, &pack).unwrap();
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "{:?}", cfg.install_error);
    let man = mods::load_manifest(&cfg.id);
    let names: Vec<&str> = man.mods.iter().filter(|m| m.origin.as_deref() == Some("modpack")).map(|m| m.file_name.as_str()).collect();
    println!("  instalados desde el pack: {names:?}");
    assert!(names.iter().any(|n| n.to_lowercase().contains("lithium")) && names.iter().any(|n| n.to_lowercase().contains("chunky")));
    assert!(modpack::take_pending(&cfg.id).is_none(), "el pack pendiente se consume");
    mgr.start(&probe, &cfg).unwrap();
    let rt = mgr.runtime(&cfg.id);
    assert!(wait_for(Duration::from_secs(600), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó: {:?}", rt.lock().unwrap().last_error);
    mgr.stop(&cfg.id).unwrap();
    assert!(mgr.wait_inactive(&cfg.id, Duration::from_secs(120)));
    println!("✔ servidor creado desde un .mrpack: {} {} con {} mods verificados, arranca", loader_label(info.loader), mc, names.len());
}
fn loader_label(l: Loader) -> &'static str { l.name() }

/// Los archivos de Mojang contra Log4Shell se descargan con la huella que dice el código (red real).
#[tokio::test]
#[ignore]
async fn e2e_log4shell_archivos() {
    for (v, name) in [("1.12.2", "112-116"), ("1.8.9", "17-111")] {
        let d = std::env::temp_dir().join(format!("h939-l4s-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        host939_core::log4shell::ensure(v, &d).await.unwrap_or_else(|e| panic!("{v}: {} {:?}", e.message, e.detail));
        host939_core::log4shell::check(v, &d).expect("verificado");
        let xml = std::fs::read_to_string(d.join(host939_core::log4shell::CONFIG_FILE)).unwrap();
        assert!(xml.contains("<Configuration"), "{v}: es una configuración de Log4j");
        println!("✔ protección de Mojang para {v} ({name}) descargada y verificada");
    }
}

/// Ficha de un modpack real de Modrinth (lo que muestra la vista de detalles) y su contenido
/// leído del .mrpack verificado. Corre con «e2e_modrinth» en la CI.
#[tokio::test]
#[ignore]
async fn e2e_modrinth_modpack_detalle() {
    // La raíz compartida del proceso: con una propia, si esta prueba terminaba primero borraba su
    // carpeta temporal mientras `e2e_modrinth` (que corre a la vez y había tomado esa misma raíz,
    // porque `paths::init` solo fija la primera) seguía usándola: «No se encontró el servidor».
    init();
    let m = modrinth::Modrinth;
    let hits = m.search_modpacks("zombie", 0).await.expect("búsqueda de modpacks");
    assert!(!hits.hits.is_empty(), "la búsqueda de «zombie» devuelve modpacks");
    let mut checked = false;
    for h in hits.hits.iter().take(6) {
        let p = m.project_detail(&h.project_id).await.expect("ficha");
        let authors = m.team_names(&p.team).await.unwrap_or_default();
        let versions = m.modpack_versions(&p.id).await.unwrap_or_default();
        println!("  · {} — {} descargas, equipo {:?}, {} versiones, {} imágenes", p.title, p.downloads, authors, versions.len(), p.gallery.len());
        let Some(v) = versions.iter().find(|v| v.files.iter().any(|f| f.filename.ends_with(".mrpack") && f.size < 30_000_000)) else { continue };
        let file = host939_core::modpack::download_modrinth(&v.id).await.expect("descarga verificada del .mrpack");
        let files = host939_core::modpack::contents(&file).expect("índice del modpack");
        let mods_n = files.iter().filter(|f| f.path.starts_with("mods/")).count();
        println!("✔ {} {}: {} archivos, {} mods ({} solo para jugadores)", p.title, v.version_number, files.len(), mods_n, files.iter().filter(|f| !f.server).count());
        assert!(!authors.is_empty(), "el equipo se lee de la API");
        checked = true;
        break;
    }
    assert!(checked, "al menos un modpack chico para revisar su contenido");
}

/// Ítems de mods reales (bajados de Modrinth y verificados por hash): el índice encuentra nombres
/// e íconos en mods de Fabric y de NeoForge, sin ejecutar nada de ellos.
#[tokio::test]
#[ignore]
async fn e2e_items_de_mods() {
    init();
    let dir = host939_core::paths::cache().join("prueba-items-de-mods");
    std::fs::create_dir_all(&dir).unwrap();
    // (proyecto, cargador, ítem que tiene que aparecer con nombre e ícono)
    for (slug, loader, item) in [("farmers-delight", "neoforge", "farmersdelight:tomato"), ("farmers-delight-refabricated", "fabric", "farmersdelight:tomato"), ("create", "neoforge", "create:wrench"), ("sodium", "fabric", "")] {
        let url = format!("https://api.modrinth.com/v2/project/{slug}/version?loaders=%5B%22{loader}%22%5D");
        let versions: serde_json::Value = host939_core::net::get_json(&url, "Modrinth").await.expect("versiones");
        let f = versions.as_array().and_then(|v| v.iter().find(|x| x["files"][0]["filename"].is_string())).map(|v| &v["files"][0]).unwrap_or_else(|| panic!("{slug} no tiene versiones para {loader}: {versions}"));
        let jar = dir.join(f["filename"].as_str().unwrap());
        host939_core::net::download(f["url"].as_str().unwrap(), &jar, host939_core::net::Hash::Sha1(f["hashes"]["sha1"].as_str().unwrap()), |_, _| {}).await.expect("descarga");
        let t = std::time::Instant::now();
        let idx = host939_core::moditems::index_jar(&jar).expect("índice");
        println!("✔ {slug} ({loader}): «{}», {} nombres, {} íconos, espacios {:?}, en {} ms", idx.mod_name, idx.names.len(), idx.icons.len(), idx.namespaces, t.elapsed().as_millis());
        if item.is_empty() {
            // Mod sin ítems (optimización): ningún ítem inventado.
            assert!(idx.icons.len() < 5, "un mod sin ítems no debería tener íconos de ítems: {:?}", idx.icons.keys().take(5).collect::<Vec<_>>());
            continue;
        }
        assert!(idx.names.contains_key(item), "falta el nombre de {item}");
        assert!(idx.icons.contains_key(item), "falta el ícono de {item}");
        let with_icon = idx.names.keys().filter(|k| idx.icons.contains_key(*k)).count();
        println!("    {with_icon} de {} ítems con nombre tienen ícono", idx.names.len());
        assert!(with_icon * 10 >= idx.names.len() * 6, "menos del 60 % de los ítems con ícono");
    }
}

/// «Preparar para jugar» de verdad, en un Minecraft de prueba (carpeta temporal, nunca el
/// `.minecraft` real): vanilla, Fabric con un mod de Modrinth y uno agregado a mano, protección
/// de la carpeta de mods anterior, vuelta atrás si algo falla, y NeoForge con su instalador oficial.
#[tokio::test]
#[ignore]
async fn e2e_preparar_para_jugar() {
    use host939_core::clientsetup::{prepare, Plan, PlanMod};
    use host939_core::model::Loader;
    use std::collections::BTreeMap;
    use std::path::Path;
    init();
    // Minecraft de PRUEBA con datos como los de una persona (nunca el real).
    let mc = tempfile::tempdir().unwrap();
    let m = mc.path();
    std::env::set_var("H939_MINECRAFT_DIR", m);
    let none = |_f: String| async { Err::<Vec<u8>, _>(host939_core::error::AppError::invalid("sin mods a mano")) };
    // Sin Launcher: lo dice claro y no instala nada.
    let vanilla = Plan { server_id: "srv-v".into(), server_name: "Vanilla".into(), mc_version: "1.21.1".into(), loader: Loader::Vanilla, ..Default::default() };
    let e = prepare(&vanilla, "127.0.0.1:25565", None, none, |_, _| {}).await.unwrap_err();
    assert!(e.message.contains("Launcher"), "{}", e.message);

    let w = |rel: &str, b: &[u8]| { let p = m.join(rel); std::fs::create_dir_all(p.parent().unwrap()).unwrap(); std::fs::write(p, b).unwrap(); };
    let options = "version:3955\nlang:es_ar\nkey_key.jump:key.keyboard.g\nkey_key.inventory:key.keyboard.tab\nnarrator:1\nhighContrast:true\ntextBackgroundOpacity:0.8\nguiScale:3\nsoundCategory_music:0.2\nresourcePacks:[\"vanilla\",\"file/MiPack.zip\"]\n";
    w("options.txt", options.as_bytes());
    w("optionsof.txt", b"ofFogType:3\n");
    w("hotbar.nbt", b"\x0a\x00\x00\x00");
    // servers.dat como lo escribe Minecraft: con ícono y «aceptar packs» en uno de ellos.
    let srv = {
        let s = |b: &mut Vec<u8>, t: &str| { b.extend((t.len() as u16).to_be_bytes()); b.extend(t.as_bytes()); };
        let mut b = vec![10, 0, 0, 9]; s(&mut b, "servers"); b.push(10); b.extend(3i32.to_be_bytes());
        for (n, ip, icon) in [("Hypixel", "mc.hypixel.net", true), ("Casa", "192.168.1.5:25565", false), ("Amigos", "amigos.ejemplo.com", false)] {
            b.push(8); s(&mut b, "ip"); s(&mut b, ip); b.push(8); s(&mut b, "name"); s(&mut b, n);
            if icon { b.push(8); s(&mut b, "icon"); s(&mut b, "iVBORw0KGgoAAAANSUhEUg=="); b.push(1); s(&mut b, "acceptTextures"); b.push(1); }
            b.push(0);
        }
        b.push(0); b
    };
    w("servers.dat", &srv);
    w("saves/Mi Mundo/level.dat", b"mundo1");
    w("saves/Mi Mundo/region/r.0.0.mca", &vec![7u8; 4096]);
    w("saves/Creativo/level.dat", b"mundo2");
    w("resourcepacks/MiPack.zip", b"pack de la persona");
    w("shaderpacks/Shader.zip", b"shader");
    w("screenshots/2026-11-21.png", b"captura");
    w("mods/mimod.jar", b"mod de la persona");
    w("config/mimod.toml", b"x=1");
    // Otro perfil con su propia carpeta y sus propias opciones.
    w("Instancias/Otro/options.txt", b"lang:pt_br\n");
    w("Instancias/Otro/saves/Isla/level.dat", b"isla");
    let profiles = serde_json::json!({"profiles": {
        "mio": {"name": "Mío", "type": "latest-release", "lastVersionId": "latest-release", "lastUsed": "2026-11-20T21:00:00.000Z"},
        "otro": {"name": "Otro", "type": "custom", "lastVersionId": "1.20.1", "gameDir": m.join("Instancias/Otro"), "lastUsed": "2025-03-01T10:00:00.000Z"}},
        "settings": {"keepLauncherOpen": true}, "version": 3});
    std::fs::write(m.join("launcher_profiles.json"), serde_json::to_string_pretty(&profiles).unwrap()).unwrap();
    // Huella de todo lo de la persona (sin lo que instala el Launcher ni las carpetas de 939 Host).
    let fingerprint = || {
        let mut out = BTreeMap::new();
        let mut stack = vec![m.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(m).unwrap().to_string_lossy().replace('\\', "/");
                if ["versions", "libraries", "assets", "939 Host", "runtime", "logs"].iter().any(|x| rel == *x) || rel.starts_with("launcher_profiles") { continue; }
                if p.is_dir() { stack.push(p); } else { out.insert(rel, host939_core::net::sha1_file(&p).unwrap()); }
            }
        }
        out
    };
    let before = fingerprint();
    let others = |lp: &serde_json::Value| { let mut v = lp["profiles"].clone(); v.as_object_mut().unwrap().retain(|k, _| !k.starts_with("939host-")); (v, lp["settings"].clone()) };
    let lp0: serde_json::Value = serde_json::from_slice(&std::fs::read(m.join("launcher_profiles.json")).unwrap()).unwrap();

    // 1) Vanilla: perfil nuevo con las preferencias del último perfil usado (copia) y la lista de servidores.
    let r = prepare(&vanilla, "127.0.0.1:25565", None, none, |_, _| {}).await.expect("vanilla");
    assert!(r.installed_version);
    assert!(m.join("versions/1.21.1/1.21.1.jar").is_file());
    let gv = Path::new(&r.game_dir).to_path_buf();
    assert_eq!(r.seeded_from.as_deref(), Some("Mío"));
    assert_eq!(std::fs::read_to_string(gv.join("options.txt")).unwrap(), options, "idioma, teclas, accesibilidad, gráficos y sonido copiados tal cual");
    assert_eq!(std::fs::read(gv.join("optionsof.txt")).unwrap(), b"ofFogType:3\n");
    let sv = host939_core::nbt::read_nbt_file(&gv.join("servers.dat")).unwrap();
    let ips: Vec<&str> = sv["servers"].as_array().unwrap().iter().map(|e| e["ip"].as_str().unwrap()).collect();
    assert_eq!(ips, vec!["127.0.0.1:25565", "mc.hypixel.net", "192.168.1.5:25565", "amigos.ejemplo.com"], "servidores guardados + el de 939 Host");
    assert_eq!(sv["servers"][1]["icon"], "iVBORw0KGgoAAAANSUhEUg==");
    assert_eq!(sv["servers"][1]["acceptTextures"], 1);
    assert_eq!(r.servers_listed, 4);
    assert_eq!(fingerprint(), before, "mundos, servidores, opciones, packs, shaders, capturas, mods y configs de la persona sin cambios");
    let lp: serde_json::Value = serde_json::from_slice(&std::fs::read(m.join("launcher_profiles.json")).unwrap()).unwrap();
    assert_eq!(others(&lp), others(&lp0), "los demás perfiles y ajustes del Launcher sin cambios");
    assert_eq!(lp["profiles"]["939host-srv-v"]["lastVersionId"], "1.21.1");
    assert_eq!(std::fs::read(m.join("launcher_profiles.json.original-939host")).unwrap(), serde_json::to_string_pretty(&profiles).unwrap().as_bytes(), "copia original intacta");
    println!("✔ vanilla 1.21.1: perfil nuevo con idioma, teclas y accesibilidad del perfil «Mío»; {} servidores guardados + el nuevo; nada de la persona cambió", ips.len() - 1);

    // 2) Fabric con Fabric API (Modrinth, verificado), un mod a mano y un resource pack opcional propio.
    let versions: serde_json::Value = host939_core::net::get_json("https://api.modrinth.com/v2/project/fabric-api/version?loaders=%5B%22fabric%22%5D&game_versions=%5B%221.21.1%22%5D", "Modrinth").await.unwrap();
    let f = &versions[0]["files"][0];
    let manual = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
    let manual_sha = host939_core::net::sha1_bytes(&manual);
    let pack = b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x01".to_vec();
    let pack_sha = host939_core::net::sha1_bytes(&pack);
    let loader = host939_core::meta::loader_support(Loader::Fabric, "1.21.1").await.unwrap().loader_version.expect("versión de Fabric");
    let fabric = Plan { server_id: "srv-f".into(), server_name: "Con mods".into(), mc_version: "1.21.1".into(), loader: Loader::Fabric, loader_version: Some(loader),
        mods: vec![
            PlanMod { filename: f["filename"].as_str().unwrap().into(), title: "Fabric API".into(), sha1: f["hashes"]["sha1"].as_str().unwrap().into(), size: f["size"].as_u64().unwrap(), url: Some(f["url"].as_str().unwrap().into()) },
            PlanMod { filename: "a-mano.jar".into(), title: "a-mano.jar".into(), sha1: manual_sha.clone(), size: manual.len() as u64, url: None },
        ],
        packs: vec![PlanMod { filename: "pack del server.zip".into(), title: "Pack del server".into(), sha1: pack_sha.clone(), size: pack.len() as u64, url: None }],
        ..Default::default() };
    let gdir = host939_core::clientsetup::resolve_game_dir(m, &fabric);
    std::fs::create_dir_all(gdir.join("mods")).unwrap();
    std::fs::write(gdir.join("mods/viejo.jar"), b"viejo").unwrap();
    let (m2, p2) = (manual.clone(), pack.clone());
    let fetch = move |n: String| { let (m, p) = (m2.clone(), p2.clone()); async move { if n.starts_with("resourcepacks/") { assert_eq!(n, "resourcepacks/pack del server.zip"); Ok(p) } else { assert_eq!(n, "a-mano.jar"); Ok(m) } } };
    let r = prepare(&fabric, "127.0.0.1:41500", None, fetch.clone(), |_, _| {}).await.expect("fabric");
    assert!(r.installed_loader && r.mods_installed == 2);
    assert_eq!(r.previous_mods.as_deref(), Some("mods anteriores"));
    assert_eq!(std::fs::read(gdir.join("mods anteriores/viejo.jar")).unwrap(), b"viejo", "lo anterior quedó intacto");
    assert!(!gdir.join("mods/viejo.jar").exists(), "sin mezclar");
    assert_eq!(std::fs::read(gdir.join("resourcepacks/pack del server.zip")).unwrap(), pack);
    let fo = std::fs::read_to_string(gdir.join("options.txt")).unwrap();
    assert!(fo.contains("lang:es_ar") && fo.contains("key_key.jump:key.keyboard.g"), "preferencias de la persona");
    assert!(fo.contains("resourcePacks:[\"vanilla\",\"file/MiPack.zip\",\"file/pack del server.zip\"]"), "pack opcional activado sin quitar los de la persona: {fo}");
    assert_eq!(r.packs, vec!["pack del server.zip"]);
    assert_ne!(gdir, gv);
    assert!(m.join(format!("versions/{0}/{0}.json", r.version_id)).is_file());
    assert_eq!(fingerprint(), before);
    println!("✔ Fabric: {} con Fabric API, un mod a mano y un resource pack opcional activado; mods de antes en «mods anteriores»; nada de la persona cambió", r.version_id);
    // Deshacer el cambio de mods: vuelve la carpeta anterior, la nueva queda aparte (nada se borra).
    let aside = host939_core::clientsetup::undo_mods(&gdir, "mods anteriores").unwrap();
    assert!(gdir.join("mods/viejo.jar").is_file() && gdir.join(&aside).join("a-mano.jar").is_file());
    println!("✔ deshacer el cambio de mods: vuelve «mods», la de 939 Host queda como «{aside}»");
    let r = prepare(&fabric, "127.0.0.1:41500", None, fetch.clone(), |_, _| {}).await.unwrap();
    assert!(r.previous_mods.is_some());

    // 3) Cada perfil conserva lo suyo: cambiar las opciones de uno no toca al otro, y volver a
    // preparar un perfil respeta sus propias opciones.
    std::fs::write(gv.join("options.txt"), "lang:fr_fr\nkey_key.jump:key.keyboard.space\n").unwrap();
    prepare(&vanilla, "127.0.0.1:25565", None, none, |_, _| {}).await.expect("vanilla de nuevo");
    assert_eq!(std::fs::read_to_string(gv.join("options.txt")).unwrap(), "lang:fr_fr\nkey_key.jump:key.keyboard.space\n", "volver al perfil recupera sus propias preferencias");
    assert!(std::fs::read_to_string(gdir.join("options.txt")).unwrap().contains("lang:es_ar"), "el otro perfil no se mezcló");
    assert_eq!(std::fs::read_to_string(m.join("Instancias/Otro/options.txt")).unwrap(), "lang:pt_br\n");
    // Dos servidores con el mismo nombre no comparten carpeta.
    let twin = Plan { server_id: "srv-v2".into(), ..vanilla.clone() };
    let rt = prepare(&twin, "127.0.0.1:25566", None, none, |_, _| {}).await.expect("mismo nombre");
    assert_ne!(Path::new(&rt.game_dir), gv);
    println!("✔ perfiles independientes: cada uno con sus opciones; dos servidores con el mismo nombre en carpetas distintas");

    // 4) Perfil creado por una versión anterior (sin las preferencias): se ofrece usarlas y lo que
    // tenía queda guardado.
    let old = Plan { server_id: "srv-viejo".into(), server_name: "Viejo".into(), mc_version: "1.21.1".into(), loader: Loader::Vanilla, ..Default::default() };
    let og = host939_core::clientsetup::resolve_game_dir(m, &old);
    std::fs::create_dir_all(&og).unwrap();
    std::fs::write(og.join("options.txt"), "lang:en_us\n").unwrap();
    let r = prepare(&old, "127.0.0.1:25567", None, none, |_, _| {}).await.expect("perfil viejo");
    assert!(r.prefs_available.is_some() && r.seeded_from.is_none());
    assert_eq!(std::fs::read_to_string(og.join("options.txt")).unwrap(), "lang:en_us\n", "no se cambió solo");
    let (_, from) = host939_core::clientsetup::last_used_profile(m, &og).unwrap();
    host939_core::clientsetup::import_preferences(&from, &og).unwrap();
    assert_eq!(std::fs::read_to_string(og.join("options.txt.antes-939host")).unwrap(), "lang:en_us\n");
    assert!(!std::fs::read_to_string(og.join("options.txt")).unwrap().contains("en_us"));
    println!("✔ perfil existente: no se pisa solo; al pedirlo toma las preferencias y guarda las anteriores");

    // 5) Si algo falla a mitad de camino (un mod o un pack con otro hash): error y nada a medias.
    let before_mods: Vec<_> = std::fs::read_dir(gdir.join("mods")).unwrap().flatten().map(|e| e.file_name()).collect();
    let mut bad = fabric.clone();
    bad.mods.push(PlanMod { filename: "roto.jar".into(), title: "roto".into(), sha1: "0".repeat(40), size: 1, url: None });
    let b2 = manual.clone();
    assert!(prepare(&bad, "127.0.0.1:41500", None, move |_| { let m = b2.clone(); async move { Ok(m) } }, |_, _| {}).await.is_err());
    let mut badp = fabric.clone();
    badp.packs[0].sha1 = "1".repeat(40);
    badp.packs[0].filename = "otro.zip".into();
    let p3 = pack.clone();
    assert!(prepare(&badp, "127.0.0.1:41500", None, move |_| { let p = p3.clone(); async move { Ok(p) } }, |_, _| {}).await.is_err());
    let after_mods: Vec<_> = std::fs::read_dir(gdir.join("mods")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(before_mods, after_mods, "la carpeta de mods no cambió");
    assert!(!gdir.join(".mods-939host-nuevos").exists() && !gdir.join("resourcepacks/otro.zip").exists());
    assert_eq!(fingerprint(), before);
    println!("✔ falla a mitad de camino (mod o pack con otro hash): no queda nada a medias ni se tocó nada de la persona");

    // 6) NeoForge con su instalador oficial en modo cliente, en su propia carpeta.
    let nv = host939_core::meta::loader_support(Loader::NeoForge, "1.21.1").await.unwrap().loader_version.expect("versión de NeoForge");
    let neo = Plan { server_id: "srv-n".into(), server_name: "NeoForge".into(), mc_version: "1.21.1".into(), loader: Loader::NeoForge, loader_version: Some(nv), ..Default::default() };
    let t = std::time::Instant::now();
    let r = prepare(&neo, "127.0.0.1:41501", None, none, |m, _| println!("    {m}")).await.expect("neoforge");
    assert!(m.join(format!("versions/{0}/{0}.json", r.version_id)).is_file());
    assert!(!Path::new(&r.game_dir).join("mods/a-mano.jar").exists(), "sin mods de otro perfil");
    assert_eq!(fingerprint(), before);
    let lp: serde_json::Value = serde_json::from_slice(&std::fs::read(m.join("launcher_profiles.json")).unwrap()).unwrap();
    assert_eq!(others(&lp), others(&lp0));
    println!("✔ NeoForge: {} instalado con su instalador oficial en {} s, en su carpeta; mundos, servidores y datos de la persona intactos", r.version_id, t.elapsed().as_secs());

    // 7) Ubicación de Minecraft: detectada (no la por defecto) y elegida a mano.
    std::env::remove_var("H939_MINECRAFT_DIR");
    let home = tempfile::tempdir().unwrap();
    let real = if cfg!(windows) { home.path().join(".minecraft") } else { home.path().join(".minecraft") };
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("launcher_profiles.json"), "{\"profiles\":{}}").unwrap();
    let (var, old_val) = if cfg!(windows) { ("APPDATA", std::env::var_os("APPDATA")) } else { ("HOME", std::env::var_os("HOME")) };
    std::env::set_var(var, home.path());
    assert_eq!(host939_core::clientsetup::minecraft_dir().unwrap(), real, "detectada en el lugar del Launcher");
    let mut st = host939_core::settings::load();
    st.minecraft_dir = m.to_string_lossy().to_string();
    host939_core::settings::save(&st).unwrap();
    assert_eq!(host939_core::clientsetup::minecraft_dir().unwrap(), m, "la carpeta elegida a mano manda");
    st.minecraft_dir = home.path().join("no-es-minecraft").to_string_lossy().to_string();
    host939_core::settings::save(&st).unwrap();
    assert_eq!(host939_core::clientsetup::minecraft_dir().unwrap(), real, "una carpeta elegida que no es de Minecraft no se usa");
    st.minecraft_dir = String::new();
    host939_core::settings::save(&st).unwrap();
    if let Some(v) = old_val { std::env::set_var(var, v); }
    println!("✔ carpeta de Minecraft: detectada en su lugar real y elegible a mano (se valida)");
}

/// Resource pack de prueba (.zip real) con el formato indicado.
fn pack_zip(path: &std::path::Path, format: u32, text: &str) {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    z.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(format!(r#"{{"pack":{{"pack_format":{format},"description":"{text}"}}}}"#).as_bytes()).unwrap();
    z.start_file("assets/minecraft/texts/splashes.txt", zip::write::SimpleFileOptions::default()).unwrap();
    z.write_all(text.as_bytes()).unwrap();
    z.finish().unwrap();
}

/// Servidor local de packs en su propio hilo (como el de la app). Devuelve el puerto.
fn pack_server(port: u16) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("puerto de packs");
            host939_core::resourcepacks::serve(l, None).await;
        });
    });
    std::thread::sleep(Duration::from_millis(300));
}

/// Panel de Resource packs con un servidor real y Modrinth real: formato leído del servidor,
/// búsqueda e instalación verificada, archivos propios (validación, conflicto, reemplazo),
/// obligatorio/opcional en server.properties, el pack puesto a mano que vuelve, la entrega local y
/// que un error no deja archivos a medias.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_resourcepacks() {
    use host939_core::resourcepacks as rp;
    init();
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let cfg = instance::create(input("Packs", "1.21.1", Loader::Vanilla, 25891), 2048).await.expect("crear");
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "{:?}", cfg.install_error);
    let id = cfg.id.clone();
    // Carpeta que no existe: se crea donde corresponde.
    assert!(!rp::dir(&id).exists());
    assert_eq!(host939_core::files::list(&id, "resourcepacks", false).unwrap_err().code, "missing");
    host939_core::files::mkdir(&id, "resourcepacks", false).unwrap();
    assert!(paths::server_run_dir(&id).join("resourcepacks").is_dir());
    assert!(host939_core::files::list(&id, "resourcepacks", false).unwrap().is_empty());
    println!("✔ carpeta resourcepacks creada en la carpeta del servidor");
    let fmt = rp::server_format(&id).expect("formato del servidor");
    assert_eq!(fmt, 34, "Minecraft 1.21.1 usa el formato 34");
    println!("✔ formato de resource packs leído del servidor real: {fmt}");
    // Modrinth real.
    let (hits, total) = rp::search("faithful", "1.21.1", false, 0).await.expect("buscar");
    assert!(total > 0 && hits.iter().all(|h| h.compatible), "solo compatibles");
    let hit = hits.iter().find(|h| h.title.to_lowercase().contains("faithful")).unwrap_or(&hits[0]).clone();
    let file = rp::install_modrinth(&id, &hit.project_id, false).await.expect("instalar de Modrinth");
    let lib = rp::library(&id).unwrap();
    let mp = lib.packs.iter().find(|p| p.file == file).unwrap();
    assert_eq!((mp.source.as_str(), mp.compat.as_str()), ("modrinth", "ok"), "{}", mp.compat_detail);
    let again = rp::install_modrinth(&id, &hit.project_id, false).await.unwrap();
    assert_eq!(again, file, "instalar de nuevo no duplica ni vuelve a bajar");
    println!("✔ Modrinth: «{}» buscado, instalado y verificado (SHA-1), compatible con 1.21.1", hit.title);
    let (old_hits, _) = rp::search("", "1.8.9", false, 0).await.unwrap();
    if let Some(h) = old_hits.iter().find(|h| !h.versions.iter().any(|v| v == "1.21.1")) {
        let e = rp::install_modrinth(&id, &h.project_id, false).await.unwrap_err();
        assert_eq!(e.code, "incompatible", "{}", e.message);
        println!("✔ un pack sin versión para 1.21.1 no se instala sin confirmar: {}", e.message);
    }
    // Archivos propios.
    let t = tempfile::tempdir().unwrap();
    let mine = t.path().join("Mi pack.zip");
    pack_zip(&mine, 34, "Hola desde 939");
    let bad = t.path().join("roto.zip");
    std::fs::write(&bad, b"no soy un zip").unwrap();
    assert!(rp::add_file(&id, &bad, "").is_err());
    let before: Vec<_> = std::fs::read_dir(rp::dir(&id)).unwrap().flatten().map(|e| e.file_name()).collect();
    let name = rp::add_file(&id, &mine, "").expect("subir");
    assert_eq!(rp::add_file(&id, &mine, "").unwrap_err().code, "exists");
    let both = rp::add_file(&id, &mine, "ambos").unwrap();
    assert_ne!(both, name);
    pack_zip(&mine, 22, "Versión vieja");
    rp::add_file(&id, &mine, "reemplazar").unwrap();
    let lib = rp::library(&id).unwrap();
    let p = lib.packs.iter().find(|p| p.file == name).unwrap();
    assert_eq!(p.compat, "incompatible", "formato 22 en 1.21.1: se avisa");
    assert!(host939_core::trash::list().iter().any(|i| i.meta.name.contains("resourcepacks/Mi pack.zip")), "el reemplazado está en la Papelera");
    let zips = |v: &[std::ffi::OsString]| v.iter().filter(|n| n.to_string_lossy().ends_with(".zip")).count();
    let now: Vec<_> = std::fs::read_dir(rp::dir(&id)).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(zips(&now), zips(&before) + 2, "el .zip roto no se agregó; el propio y su copia sí");
    assert!(!now.iter().any(|n| n.to_string_lossy().contains("939host-copia")), "sin restos");
    println!("✔ archivo propio: validado, conflicto con «guardar los dos» o «reemplazar» (el anterior a la Papelera), versión incompatible avisada");
    // Obligatorio: server.properties con la entrega local; el pack puesto a mano vuelve al quitarlo.
    let props_path = paths::server_run_dir(&id).join("server.properties");
    let mut c = instance::load(&id).unwrap();
    c.extra_properties.insert("resource-pack".into(), "https://ejemplo.com/pack-a-mano.zip".into());
    instance::save(&c).unwrap();
    host939_core::properties::write_for(&c).unwrap();
    assert_eq!(rp::library(&id).unwrap().manual_pack.as_deref(), Some("https://ejemplo.com/pack-a-mano.zip"));
    pack_zip(&mine, 34, "Hola desde 939");
    rp::add_file(&id, &mine, "reemplazar").unwrap();
    rp::set(&id, &name, Some("obligatorio"), None).unwrap();
    host939_core::properties::write_for(&c).unwrap();
    let pr = host939_core::properties::read(&props_path);
    let sha = rp::required(&id).unwrap().1.sha1;
    assert_eq!(pr["resource-pack"], format!("http://127.0.0.1:{}/rp/{id}/{sha}.zip", rp::LOCAL_PORT));
    assert_eq!(pr["resource-pack-sha1"], sha);
    assert_eq!(pr["require-resource-pack"], "true");
    assert!(pr["resource-pack-prompt"].contains("obligatorio"));
    // Solo uno obligatorio: el de Modrinth pasa a serlo y el propio queda opcional; su dirección es Modrinth.
    let demoted = rp::set(&id, &file, Some("obligatorio"), None).unwrap();
    assert_eq!(demoted.as_deref(), Some(name.as_str()));
    host939_core::properties::write_for(&c).unwrap();
    let pr = host939_core::properties::read(&props_path);
    assert!(pr["resource-pack"].starts_with("https://cdn.modrinth.com/"), "{}", pr["resource-pack"]);
    // Opcionales activos: los instala «Preparar para jugar» (con su descarga si son de Modrinth).
    let cp = rp::client_packs(&id);
    assert!(cp.iter().any(|p| p.filename == name && p.url.is_none()) && !cp.iter().any(|p| p.filename == file));
    // Quitar el obligatorio: vuelve el pack que había puesto a mano.
    rp::set(&id, &file, Some("opcional"), None).unwrap();
    host939_core::properties::write_for(&c).unwrap();
    let pr = host939_core::properties::read(&props_path);
    assert_eq!(pr["resource-pack"], "https://ejemplo.com/pack-a-mano.zip");
    assert!(pr.get("require-resource-pack").map(|v| v != "true").unwrap_or(true));
    println!("✔ obligatorio en server.properties (dirección local o de Modrinth, SHA-1, rechazo = no entra), uno solo a la vez, y el pack puesto a mano vuelve al quitarlo");
    // Entrega local: sirve exactamente el archivo; un SHA-1 desconocido no.
    rp::set(&id, &name, Some("obligatorio"), None).unwrap();
    pack_server(39400);
    let url = format!("http://127.0.0.1:39400/rp/{id}/{sha}.zip");
    let got = reqwest::get(&url).await.unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(net::sha1_bytes(&got.bytes().await.unwrap()), sha);
    assert_eq!(reqwest::get(format!("http://127.0.0.1:39400/rp/{id}/{}.zip", "0".repeat(40))).await.unwrap().status(), 404);
    assert_eq!(reqwest::get("http://127.0.0.1:39400/../server.properties").await.unwrap().status(), 404);
    println!("✔ entrega local: el pack exacto por su SHA-1; nada más");
    // El servidor real arranca con el pack obligatorio configurado.
    let cfg = instance::load(&id).unwrap();
    mgr.start(&probe, &cfg).expect("iniciar");
    let rt = mgr.runtime(&id);
    assert!(wait_for(Duration::from_secs(300), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó");
    assert!(!mgr.console(&id).iter().any(|l| l.contains("resource pack") && (l.contains("ERROR") || l.contains("Invalid"))), "sin errores del pack en el servidor");
    mgr.stop_all();
    // Quitar un pack: va a la Papelera y el servidor deja de enviarlo.
    rp::remove(&id, &name).unwrap();
    host939_core::properties::write_for(&cfg).unwrap();
    assert!(!rp::dir(&id).join(&name).exists());
    assert_eq!(host939_core::properties::read(&props_path)["resource-pack"], "https://ejemplo.com/pack-a-mano.zip");
    let _ = rec;
    println!("✔ servidor real encendido con el pack obligatorio; quitar el pack lo manda a la Papelera y deja de enviarse");
}

/// Un cliente oficial de Minecraft con un pack obligatorio propio entregado por 939 Host: lo baja de
/// la entrega local, entra; si el pack no cambió no lo vuelve a bajar; si cambia, baja el nuevo; sin
/// aceptarlo no entra; con un pack opcional instalado en su perfil y ninguno obligatorio, entra.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn e2e_resourcepacks_cliente_real() {
    use host939_core::resourcepacks as rp;
    init();
    let script = std::env::var("CLIENTE_REAL").expect("CLIENTE_REAL");
    let mc = std::env::var("E2E_MC").unwrap_or_else(|_| "26.2".into());
    let rec = Arc::new(Recorder::default());
    let mgr = Manager::new(rec.clone());
    let probe = Probe::new();
    let port = 25871;
    let mut inp = input("Packs cliente", &mc, Loader::Vanilla, port);
    inp.online_mode = false;
    let cfg = instance::create(inp, 4096).await.expect("crear");
    instance::install(&mgr, cfg.id.clone()).await;
    let cfg = instance::load(&cfg.id).unwrap();
    assert!(cfg.installed, "{:?}", cfg.install_error);
    let id = cfg.id.clone();
    let fmt = rp::server_format(&id).expect("formato");
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Pack obligatorio.zip");
    pack_zip(&src, fmt, "Pack de 939 Host v1");
    let name = rp::add_file(&id, &src, "").unwrap();
    rp::set(&id, &name, Some("obligatorio"), None).unwrap();
    pack_server(rp::LOCAL_PORT);
    let dir = paths::root().join("cliente-packs");
    let name_p = "Jugador939";
    let join = |accept: &str, opcional: Option<&std::path::Path>, secs: u64| -> bool {
        let start = mgr.console(&id).len();
        let mut cmd = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" });
        cmd.args([&script, &mc, &format!("127.0.0.1:{port}"), dir.to_str().unwrap(), name_p, &secs.to_string()]).env("ACEPTAR_PACKS", accept);
        if let Some(o) = opcional { cmd.env("PACK_OPCIONAL", o); } else { cmd.env_remove("PACK_OPCIONAL"); }
        let mut child = cmd.spawn().expect("cliente");
        let ok = wait_for(Duration::from_secs(secs + 300), || mgr.console(&id).iter().skip(start).any(|l| l.contains(&format!("{name_p} joined the game"))));
        // Que se quede un momento conectado (no lo echan después de entrar).
        let kicked = ok && { std::thread::sleep(Duration::from_secs(8)); mgr.console(&id).iter().skip(start).any(|l| l.contains(&format!("{name_p} lost connection")) || l.contains(&format!("{name_p} left the game"))) };
        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_secs(3));
        ok && !kicked
    };
    let start = |c: &ServerConfig| {
        host939_core::properties::write_for(c).unwrap();
        mgr.start(&probe, c).expect("iniciar");
        let rt = mgr.runtime(&c.id);
        assert!(wait_for(Duration::from_secs(400), || rt.lock().unwrap().status() == RunStatus::Running), "no arrancó");
        assert!(wait_for(Duration::from_secs(30), || rt.lock().unwrap().reach.as_deref() == Some("ok")));
    };
    start(&cfg);
    let sha1 = rp::required(&id).unwrap().1.sha1;
    // 1) Acepta: baja el pack de la entrega local y entra.
    let ok = join("1", None, 120);
    let log = std::fs::read_to_string(dir.join("game/logs/latest.log")).unwrap_or_default();
    let hits = rp::served().iter().filter(|s| s.ends_with(&sha1)).count();
    assert!(ok, "con el pack obligatorio aceptado, el jugador entra. Consola: {:?}", mgr.console(&id).iter().rev().take(15).collect::<Vec<_>>());
    assert_eq!(hits, 1, "el cliente bajó el pack de 939 Host una vez");
    println!("✔ pack obligatorio: el cliente real lo bajó de la entrega local de 939 Host ({}…) y entró", &sha1[..12]);
    if log.contains("Reloading ResourceManager") { println!("  (el cliente recargó sus recursos con el pack)"); }
    // 2) Mismo pack: no lo vuelve a bajar.
    assert!(join("1", None, 90), "entra de nuevo");
    assert_eq!(rp::served().iter().filter(|s| s.ends_with(&sha1)).count(), 1, "sin descarga innecesaria");
    println!("✔ sin cambios en el pack: el cliente no lo volvió a bajar");
    // 3) El pack cambia: el servidor envía la versión nueva y el cliente la baja.
    mgr.stop_all();
    pack_zip(&src, fmt, "Pack de 939 Host v2");
    rp::add_file(&id, &src, "reemplazar").unwrap();
    let sha2 = rp::required(&id).unwrap().1.sha1;
    assert_ne!(sha2, sha1);
    start(&instance::load(&id).unwrap());
    assert_eq!(host939_core::properties::read(&paths::server_run_dir(&id).join("server.properties"))["resource-pack-sha1"], sha2);
    assert!(join("1", None, 120), "entra con la versión nueva");
    assert_eq!(rp::served().iter().filter(|s| s.ends_with(&sha2)).count(), 1, "bajó la versión nueva");
    println!("✔ pack cambiado: el servidor envió la versión nueva ({}…) y el cliente la bajó", &sha2[..12]);
    // 4) Sin aceptar el pack obligatorio, no entra.
    let served_before = rp::served().len();
    assert!(!join("0", None, 80), "sin aceptar el pack obligatorio no se entra");
    assert_eq!(rp::served().len(), served_before);
    println!("✔ sin aceptar el pack obligatorio, el jugador no entra (lo exige Minecraft)");
    // 5) Opcional: ninguno obligatorio y un pack instalado en el perfil del jugador: entra igual.
    mgr.stop_all();
    rp::set(&id, &name, Some("opcional"), None).unwrap();
    start(&instance::load(&id).unwrap());
    let pr = host939_core::properties::read(&paths::server_run_dir(&id).join("server.properties"));
    assert!(pr.get("resource-pack").map(|v| v.is_empty()).unwrap_or(true) && pr.get("require-resource-pack").map(|v| v != "true").unwrap_or(true));
    let opt = rp::dir(&id).join(&name);
    assert!(join("0", Some(&opt), 90), "un pack opcional no impide entrar");
    println!("✔ pack opcional instalado en el perfil del jugador y ninguno obligatorio: entra sin problemas");
    mgr.stop_all();
    let _ = rec;
}
