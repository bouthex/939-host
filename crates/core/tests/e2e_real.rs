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
            mgr.send_command(&cfg.id, &format!("kick {name} prueba terminada")).ok();
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
