//! Nodo de 939 Host sin interfaz, para pruebas con varios procesos (cada uno es "otra PC",
//! con su propia carpeta de datos e identidad). Lee órdenes por la entrada estándar y responde
//! una línea `@@ <json>` por orden. Los avisos salen como `## <texto>`.
//!
//! Modos:
//! * normal: servidores de Minecraft reales (Java + jar de Mojang).
//! * `--fake`: un "servidor" mínimo que solo responde al estado de Minecraft con el nombre de la
//!   PC; sirve para probar el protocolo (traspaso, caída, aislamiento, sincronización, ruteo) rápido.

use host939_cluster::{GroupNode, LocalHost, Timing};
use host939_core::events::NoEvents;
use host939_core::hardware::Probe;
use host939_core::model::{CreateServerInput, Loader, RunStatus};
use host939_core::process::Manager;
use host939_core::snapshots::{self, SnapshotInfo};
use host939_core::{identity, instance, paths};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Local {
    mgr: Manager,
    probe: Probe,
    fake: bool,
    name: String,
    fakes: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    rt: tokio::runtime::Handle,
}

fn say(tag: &str, s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{tag} {s}");
    let _ = o.flush();
}

impl LocalHost for Local {
    fn is_running(&self, server: &str) -> bool {
        if self.fake { return self.fakes.lock().unwrap().contains_key(server); }
        matches!(self.mgr.runtime(server).lock().unwrap().status(), RunStatus::Running | RunStatus::Starting)
    }
    fn mc_port(&self, server: &str) -> u16 { instance::load(server).map(|c| c.port).unwrap_or(25565) }
    fn prepare(&self, server: &str) -> Result<(), String> {
        let cfg = instance::load(server).map_err(|e| e.message)?;
        if cfg.installed || self.fake { return Ok(()); }
        self.rt.block_on(instance::install(&self.mgr, server.to_string()));
        let cfg = instance::load(server).map_err(|e| e.message)?;
        if cfg.installed { Ok(()) } else { Err(cfg.install_error.unwrap_or_else(|| "instalación fallida".into())) }
    }
    fn start(&self, server: &str) -> Result<(), String> {
        let cfg = instance::load(server).map_err(|e| e.message)?;
        if self.fake {
            host939_core::lease::can_start(server).map_err(|e| e.message)?;
            let run = paths::server_run_dir(server);
            std::fs::create_dir_all(run.join("world")).map_err(|e| e.to_string())?;
            if !run.join("world/level.dat").exists() { std::fs::write(run.join("world/level.dat"), b"nuevo").map_err(|e| e.to_string())?; }
            let l = std::net::TcpListener::bind(("127.0.0.1", cfg.port)).map_err(|e| format!("puerto: {e}"))?;
            l.set_nonblocking(true).unwrap();
            let name = self.name.clone();
            let _g = self.rt.enter();
            let task = self.rt.spawn(async move {
                let l = tokio::net::TcpListener::from_std(l).unwrap();
                loop {
                    let Ok((c, _)) = l.accept().await else { break };
                    let n = name.clone();
                    tokio::spawn(async move { let _ = host939_cluster::moving::respond(c, "FAKE", &format!("hosteado por {n}")).await; });
                }
            });
            self.fakes.lock().unwrap().insert(server.to_string(), task);
            return Ok(());
        }
        self.mgr.start(&self.probe, &cfg).map_err(|e| e.message)?;
        let rt = self.mgr.runtime(server);
        let end = Instant::now() + Duration::from_secs(400);
        while Instant::now() < end {
            match rt.lock().unwrap().status() {
                RunStatus::Running => return Ok(()),
                RunStatus::Crashed | RunStatus::Stopped => return Err("el servidor se detuvo al iniciar".into()),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        Err("no terminó de iniciar".into())
    }
    fn stop(&self, server: &str) -> Result<(), String> {
        if self.fake {
            if let Some(t) = self.fakes.lock().unwrap().remove(server) { t.abort(); }
            std::thread::sleep(Duration::from_millis(100));
            return Ok(());
        }
        if !self.mgr.runtime(server).lock().unwrap().is_active() { return Ok(()); }
        self.mgr.stop(server).map_err(|e| e.message)?;
        if self.mgr.wait_inactive(server, Duration::from_secs(120)) { Ok(()) } else { Err("no se detuvo a tiempo".into()) }
    }
    fn snapshot(&self, server: &str, label: &str, kind: &str) -> Result<SnapshotInfo, String> {
        let mc = instance::load(server).map(|c| c.mc_version).unwrap_or_default();
        snapshots::take(&self.mgr, server, label, kind, &mc).map_err(|e| e.message)
    }
    fn profile(&self) -> host939_cluster::HostProfile {
        host939_cluster::HostProfile { cpu: "CPU de prueba".into(), cores: 4, ram_total_mb: std::env::var("H939_RAM").ok().and_then(|s| s.parse().ok()).unwrap_or(16384), ram_free_mb: 8192, cpu_score: Some(self.score()), disk_free_gb: Some(100.0) }
    }
    fn score(&self) -> u32 { std::env::var("H939_SCORE").ok().and_then(|s| s.parse().ok()).unwrap_or(100) }
    fn allow_hosting(&self, _: &str) -> bool { std::env::var("H939_NO_HOST").is_err() }
    fn note(&self, server: &str, message: &str) { say("##", &format!("[{}] {message}", &server[..server.len().min(8)])); }
    fn changed(&self, _: &str) {}
}

mod redteam {
    //! Herramientas de la prueba ofensiva: hablan el protocolo «a mano», como lo haría un
    //! cliente modificado. Solo existen en el nodo de pruebas, no en la app.
    use host939_cluster::{cert, wire};
    use iroh::{Endpoint, EndpointAddr, EndpointId};
    use serde_json::{json, Value};
    use std::time::Duration;

    fn alpn(name: &str) -> &'static [u8] {
        match name {
            "ctrl" => host939_cluster::ALPN_CTRL, "sync" => host939_cluster::ALPN_SYNC, "mc" => host939_cluster::ALPN_MC,
            "bridge" => host939_cluster::ALPN_BRIDGE, "join" => host939_cluster::ALPN_JOIN, "files" => host939_cluster::files::ALPN_FILES,
            _ => b"939host/desconocido/9",
        }
    }
    async fn open(ep: &Endpoint, node: &str, a: &str) -> Result<(iroh::endpoint::SendStream, iroh::endpoint::RecvStream), String> {
        let id: EndpointId = node.parse().map_err(|_| "id inválido".to_string())?;
        let c = tokio::time::timeout(Duration::from_secs(10), ep.connect(EndpointAddr::new(id), alpn(a))).await.map_err(|_| "sin respuesta".to_string())?.map_err(|e| format!("conexión rechazada: {e}"))?;
        c.open_bi().await.map_err(|e| e.to_string())
    }
    pub async fn raw(ep: &Endpoint, node: &str, a: &str, body: &str) -> Result<Value, String> {
        let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let (mut s, mut r) = open(ep, node, a).await?;
        wire::write_msg(&mut s, &v).await.map_err(|e| e.to_string())?;
        let _ = s.finish();
        match tokio::time::timeout(Duration::from_secs(15), wire::read_msg::<_, Value>(&mut r)).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Ok(json!({ "closed": e.to_string() })),
            Err(_) => Ok(json!({ "closed": "sin respuesta" })),
        }
    }
    pub async fn raw_frames(ep: &Endpoint, node: &str, a: &str, body: &str, n: usize) -> Result<Value, String> {
        let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
        let (mut s, mut r) = open(ep, node, a).await?;
        wire::write_msg(&mut s, &v).await.map_err(|e| e.to_string())?;
        let _ = s.finish();
        let mut sizes = vec![];
        for _ in 0..n {
            match tokio::time::timeout(Duration::from_secs(10), wire::read_frame(&mut r, 64 * 1024 * 1024)).await {
                Ok(Ok(f)) => sizes.push(f.len()),
                _ => break,
            }
        }
        Ok(json!(sizes))
    }
    pub async fn raw_bytes(ep: &Endpoint, node: &str, a: &str, hexs: &str) -> Result<Value, String> {
        let bytes = hex::decode(hexs).map_err(|e| e.to_string())?;
        let (mut s, mut r) = open(ep, node, a).await?;
        s.write_all(&bytes).await.map_err(|e| e.to_string())?;
        let _ = s.finish();
        let got = tokio::time::timeout(Duration::from_secs(5), r.read_to_end(1 << 20)).await;
        Ok(json!({ "replyBytes": got.ok().and_then(|x| x.ok()).map(|b| b.len()).unwrap_or(0) }))
    }
    pub async fn forge(ep: &Endpoint, secret: &[u8; 32], sid: &str, to: &str, kind: &str, host: &str, epoch: Option<u64>) -> Result<Value, String> {
        let cur = host939_core::lease::read(sid).ok_or("sin permiso")?;
        let key = iroh::SecretKey::from_bytes(secret);
        let me = key.public().to_string();
        let k: host939_core::lease::LeaseKind = serde_json::from_value(json!(kind)).map_err(|e| e.to_string())?;
        let mut l = host939_core::lease::Lease {
            epoch: epoch.unwrap_or(cur.epoch + 1), host: if host == "yo" { me.clone() } else { host.to_string() }, host_name: "forjado".into(),
            state: host939_core::lease::LeaseState::Hosting, since: chrono::Local::now().to_rfc3339(), required: (0, 0), kind: k,
            issuer: me, prev: Some((cur.epoch, cur.host.clone(), cur.state)), votes: vec![], sig: None,
        };
        cert::sign(&key, sid, &mut l);
        raw(ep, to, "ctrl", &json!({ "t": "lease", "server": sid, "lease": l }).to_string()).await
    }
    pub async fn replay(ep: &Endpoint, sid: &str, to: &str, idx: usize) -> Result<Value, String> {
        let log = host939_core::lease::history(sid);
        let l = log.get(idx).ok_or("no hay ese permiso en el historial")?;
        raw(ep, to, "ctrl", &json!({ "t": "lease", "server": sid, "lease": l }).to_string()).await
    }
    pub async fn remote_info(ep: &Endpoint, node: &str) -> Result<Value, String> {
        let id: EndpointId = node.parse().map_err(|_| "id inválido".to_string())?;
        let info = ep.remote_info(id).await;
        let addrs: Vec<String> = info.map(|i| i.addrs().map(|a| format!("{:?}", a.addr())).collect()).unwrap_or_default();
        Ok(json!(addrs))
    }
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let root = arg("--root").expect("--root");
    let name = arg("--name").unwrap_or_else(|| "nodo".into());
    let fake = std::env::args().any(|a| a == "--fake");
    paths::init(root.clone().into()).expect("init");
    if let Some(j) = arg("--java25") {
        let link = paths::runtimes().join("java-25");
        #[cfg(unix)]
        if !link.exists() { let _ = std::os::unix::fs::symlink(j, link); }
        #[cfg(not(unix))]
        let _ = (j, link);
    }
    let node_id = identity::load_or_create_node().expect("identidad");
    let secret = identity::node_secret().unwrap();
    let local = Arc::new(Local { mgr: Manager::new(Arc::new(NoEvents)), probe: Probe::new(), fake, name: name.clone(), fakes: Default::default(), rt: tokio::runtime::Handle::current() });
    let timing = if std::env::var("H939_FAST").is_ok() {
        Timing { poll: Duration::from_millis(700), host_dead: Duration::from_secs(7), fence: Duration::from_secs(4), seen_window: Duration::from_secs(60), release_wait: Duration::from_secs(15), request_ttl: Duration::from_secs(std::env::var("H939_REQUEST_TTL").ok().and_then(|v| v.parse().ok()).unwrap_or(120)), ownership_wait: Duration::from_secs(std::env::var("H939_OWNERSHIP_WAIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20)) }
    } else { Timing::default() };
    // --relays: igual que la app (conexión directa y relays públicos solo como último recurso).
    // --private: modo privado (solo relays, sin transporte IP propio).
    let relays = if std::env::args().any(|a| a == "--private") { host939_p2p::Relays::Private }
        else if std::env::args().any(|a| a == "--relays") { host939_p2p::Relays::PublicFallback } else { host939_p2p::Relays::Disabled };
    let node = GroupNode::start(secret, &name, relays, local.clone(), timing).await.expect("nodo");
    assert_eq!(node.id(), node_id.node_id);
    say("##", &format!("listo {}", node.id()));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() { let Ok(l) = line else { break }; if tx.send(l).is_err() { break; } }
    });
    while let Some(line) = rx.recv().await {
        let parts: Vec<String> = line.split_whitespace().map(String::from).collect();
        if parts.is_empty() { continue; }
        let p = |i: usize| parts.get(i).cloned().unwrap_or_default();
        let res: Result<serde_json::Value, String> = async {
            match parts[0].as_str() {
                "id" => Ok(serde_json::json!(node.id())),
                "addr" => Ok(serde_json::to_value(node.addr()).unwrap()),
                "create" => {
                    let input = CreateServerInput {
                        name: p(1), description: String::new(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1536,
                        difficulty: "normal".into(), gamemode: "creative".into(), max_players: 4, port: p(2).parse().unwrap_or(25700),
                        online_mode: false, whitelist: false, view_distance: 3, simulation_distance: 3, level_seed: "939".into(),
                        accept_eula: true, import_world_path: None, optimize: vec![],
                    };
                    let cfg = instance::create(input, 8192).await.map_err(|e| e.message)?;
                    if fake {
                        let mut c = cfg.clone(); c.installed = true; c.java_major = 25; instance::save(&c).map_err(|e| e.message)?;
                    } else {
                        instance::install(&local.mgr, cfg.id.clone()).await;
                        let c = instance::load(&cfg.id).map_err(|e| e.message)?;
                        if !c.installed { return Err(format!("instalación: {:?}", c.install_error)); }
                    }
                    Ok(serde_json::json!(cfg.id))
                }
                "setport" => {
                    let mut c = instance::load(&p(1)).map_err(|e| e.message)?;
                    c.port = p(2).parse().map_err(|_| "puerto inválido".to_string())?;
                    instance::save(&c).map_err(|e| e.message)?;
                    Ok(serde_json::json!(true))
                }
                "share" => node.share(&p(1)).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "invite" => node.invite(&p(1), &p(2), "prueba").map(|t| serde_json::json!(t)).map_err(|e| e.to_string()),
                "join" => node.join(&p(1)).await.map(|s| serde_json::json!(s)).map_err(|e| e.to_string()),
                "options" => node.set_options(&p(1), p(2) == "1", p(3) == "1").map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "start" => {
                    node.prepare_start(&p(1)).await.map_err(|e| e.to_string())?;
                    let (l, s) = (local.clone(), p(1));
                    tokio::task::spawn_blocking(move || { l.prepare(&s)?; l.start(&s) }).await.unwrap()?;
                    Ok(serde_json::json!(true))
                }
                "stop" => { let (l, s) = (local.clone(), p(1)); tokio::task::spawn_blocking(move || l.stop(&s)).await.unwrap().map(|_| serde_json::json!(true)) }
                "cmd" => local.mgr.send_command(&p(1), &parts[2..].join(" ")).map(|_| serde_json::json!(true)).map_err(|e| e.message),
                "write" => {
                    // Simula juego: escribe un archivo en el mundo (solo si este nodo lo tiene encendido).
                    if !local.is_running(&p(1)) { return Err("no está encendido acá".into()); }
                    std::fs::write(paths::server_run_dir(&p(1)).join("world").join(p(2)), p(3)).map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(true))
                }
                "read" => Ok(serde_json::json!(std::fs::read_to_string(paths::server_run_dir(&p(1)).join("world").join(p(2))).ok())),
                "snapshot" => { let (l, s) = (local.clone(), p(1)); tokio::task::spawn_blocking(move || l.snapshot(&s, "prueba", "auto")).await.unwrap().map(|i| serde_json::to_value(i).unwrap()) }
                "snapshots" => Ok(serde_json::to_value(snapshots::list(&p(1))).unwrap()),
                "handover" => node.handover(&p(1), parts.get(2).map(|s| s.as_str())).await.map(|n| serde_json::json!(n)).map_err(|e| e.to_string()),
                "release" => node.release(&p(1)).await.map(|n| serde_json::json!(n)).map_err(|e| e.to_string()),
                "take" => node.take_here(&p(1)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "status" => Ok(serde_json::to_value(node.status(&p(1)).await).unwrap()),
                "route" => node.open_route(&p(1), p(2).parse().unwrap_or(0)).await.map(|port| serde_json::json!(port)).map_err(|e| e.to_string()),
                "online" => Ok(serde_json::json!(if fake { local.is_running(&p(1)) } else { local.mgr.runtime(&p(1)).lock().unwrap().status() == RunStatus::Running })),
                "running" => Ok(serde_json::json!(local.is_running(&p(1)))),
                "pid" => Ok(serde_json::json!(local.mgr.runtime(&p(1)).lock().unwrap().pid)),
                "sync" => { node.sync_now(&p(1)).await; Ok(serde_json::json!(true)) }
                "quit" => Ok(serde_json::json!(node.quit().await)),
                "rename" => { node.set_identity(&p(1), &p(2)); Ok(serde_json::json!(true)) }
                "request" => node.request_host(&p(1)).await.map(|n| serde_json::json!(n)).map_err(|e| e.to_string()),
                "cancelreq" => node.cancel_host_request(&p(1)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "answer" => node.answer_host_request(&p(1), p(2) == "si").await.map(|n| serde_json::json!(n)).map_err(|e| e.to_string()),
                // Administración remota de archivos (en la PC anfitriona).
                "fls" => node.files_list(&p(1), parts.get(2).map(|s| s.as_str()).unwrap_or("")).await.map(|v| serde_json::to_value(v).unwrap()).map_err(|e| e.to_string()),
                "fmkdir" => node.files_mkdir(&p(1), &p(2)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "frm" => node.files_delete(&p(1), &p(2)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "fmv" => node.files_transfer(&p(1), &p(2), &p(3), true, parts.get(4).map(|s| s == "reemplazar").unwrap_or(false)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "fcp" => node.files_transfer(&p(1), &p(2), &p(3), false, parts.get(4).map(|s| s == "reemplazar").unwrap_or(false)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "fget" => node.files_download(&p(1), &p(2), std::path::Path::new(&p(3)), |_, _| true).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "fput" => node.files_upload(&p(1), std::path::Path::new(&p(2)), &p(3), parts.get(4).map(|s| s == "reemplazar").unwrap_or(false), |_, _| true).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "fputcancel" => node.files_upload(&p(1), std::path::Path::new(&p(2)), &p(3), false, |sent, _| sent < 1024 * 1024).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "exists" => Ok(serde_json::json!(paths::server_run_dir(&p(1)).join(p(2)).exists())),
                "sha1" => Ok(serde_json::json!(host939_core::net::sha1_file(&paths::server_run_dir(&p(1)).join(p(2))).ok())),
                "lsdir" => Ok(serde_json::json!(std::fs::read_dir(paths::server_run_dir(&p(1)).join(p(2))).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>()).unwrap_or_default())),
                "logo" => {
                    // PNG mínimos válidos (cabecera + IHDR + IEND) de 256×256 y 64×64.
                    let png = |w: u32| { let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13]; v.extend_from_slice(b"IHDR"); v.extend_from_slice(&w.to_be_bytes()); v.extend_from_slice(&w.to_be_bytes()); v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]); v.extend_from_slice(&[0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]); v };
                    host939_core::logo::set(&p(1), "preset:3", &png(256), &png(64)).map_err(|e| e.message).and_then(|_| node.refresh_card(&p(1)).map(|b| serde_json::json!(b)).map_err(|e| e.to_string()))
                }
                "card" => Ok(host939_core::network::load_share(&p(1)).card.unwrap_or_default()),
                "invitefor" => node.invite_for(&p(1), &p(2), "prueba", p(3).parse().unwrap_or(24)).map(|t| serde_json::json!(t)).map_err(|e| e.to_string()),
                "revoke" => node.revoke_invite(&p(1), &p(2)).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "invites" => Ok(serde_json::to_value(host939_core::network::load_share(&p(1)).invites).unwrap()),
                "expireinvites" => {
                    // Solo pruebas: adelanta el vencimiento de todas las invitaciones de este nodo.
                    let mut sh = host939_core::network::load_share(&p(1));
                    for i in sh.invites.iter_mut() { i.expires_at = Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339()); }
                    host939_core::network::save_share(&p(1), &sh).map_err(|e| e.message)?;
                    Ok(serde_json::json!(true))
                }
                "share_json" => Ok(serde_json::to_value(host939_core::network::load_share(&p(1))).unwrap()),
                "remove" => node.remove_member(&p(1), &p(2)).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "setrole" => {
                    // La propietaria cambia el rol de una PC (firma una revisión nueva).
                    let mut sh = host939_core::network::load_share(&p(1));
                    for m in sh.members.iter_mut() { if m.node_id == p(2) { m.role = p(3); } }
                    node.commit_share(&p(1), &mut sh).map(|_| serde_json::json!(true)).map_err(|e| e.to_string())
                }
                "transfer" => node.transfer_ownership(&p(1), &p(2)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "reccreate" => node.create_recovery_code(&p(1)).map(|c| serde_json::json!(c)).map_err(|e| e.to_string()),
                "recrotate" => node.rotate_recovery_code(&p(1), &p(2)).map(|c| serde_json::json!(c)).map_err(|e| e.to_string()),
                "recover" => node.recover_with_code(&p(1), &p(2)).await.map(|c| serde_json::json!(c)).map_err(|e| e.to_string()),
                "resetreq" => node.request_recovery_reset(&p(1)).map(|c| serde_json::json!(c)).map_err(|e| e.to_string()),
                "resetfinish" => node.finish_recovery_reset(&p(1)).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "successor" => node.set_successor(&p(1), parts.get(2).map(|s| s.as_str())).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "succstart" => node.start_succession(&p(1)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "succcancel" => node.cancel_succession(&p(1)).map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "succcomplete" => node.complete_succession(&p(1)).await.map(|_| serde_json::json!(true)).map_err(|e| e.to_string()),
                "succstatus" => Ok(serde_json::json!(node.succession_status(&p(1)))),
                "lease" => Ok(serde_json::to_value(host939_core::lease::read(&p(1))).unwrap()),
                "leaselog" => Ok(serde_json::to_value(host939_core::lease::history(&p(1))).unwrap()),
                "audit" => Ok(serde_json::json!(std::fs::read_to_string(paths::server_dir(&p(1)).join("audit.jsonl")).unwrap_or_default())),
                "rejected" => Ok(serde_json::json!(node.rejected_count(&p(1)))),
                /* ── Red team: un nodo malicioso manda mensajes a mano (sin la lógica del grupo) ── */
                // raw <nodo> <alpn> <json>: un mensaje y su respuesta, tal cual.
                "raw" => redteam::raw(node.endpoint(), &p(1), &p(2), &parts[3..].join(" ")).await,
                // rawframes <nodo> <alpn> <json> <n>: mensaje y hasta n respuestas en tramas.
                "rawframes" => redteam::raw_frames(node.endpoint(), &p(1), &p(2), &parts[3..parts.len() - 1].join(" "), parts.last().and_then(|x| x.parse().ok()).unwrap_or(1)).await,
                // rawbytes <nodo> <alpn> <hex>: bytes arbitrarios (mensajes malformados).
                "rawbytes" => redteam::raw_bytes(node.endpoint(), &p(1), &p(2), &p(3)).await,
                // forge <sid> <destino> <tipo> <anfitriona> [época]: permiso firmado por ESTA PC con el
                // tipo y la anfitriona que se quiera, encadenado al permiso actual, enviado a <destino>.
                "forge" => redteam::forge(node.endpoint(), &secret, &p(1), &p(2), &p(3), &p(4), parts.get(5).and_then(|x| x.parse().ok())).await,
                // replay <sid> <destino> <índice>: reenvía un permiso viejo del historial.
                "replay" => redteam::replay(node.endpoint(), &p(1), &p(2), p(3).parse().unwrap_or(0)).await,
                // replaylist <sid> <destino> <archivo>: hace circular una lista firmada guardada.
                "savelist" => { std::fs::write(p(2), serde_json::to_vec(&host939_core::network::load_share(&p(1))).unwrap()).map_err(|e| e.to_string())?; Ok(serde_json::json!(true)) }
                "remoteinfo" => redteam::remote_info(node.endpoint(), &p(1)).await,
                "myaddr" => Ok(serde_json::to_value(node.addr()).unwrap()),
                "peersfile" => Ok(serde_json::json!(std::fs::read_to_string(paths::root().join("peers.json")).unwrap_or_default())),
                "exit" => { node.shutdown().await; say("@@", "{\"ok\":true,\"v\":\"bye\"}"); std::process::exit(0) }
                other => Err(format!("orden desconocida: {other}")),
            }
        }.await;
        let out = match res { Ok(v) => serde_json::json!({ "ok": true, "v": v }), Err(e) => serde_json::json!({ "ok": false, "err": e }) };
        say("@@", &out.to_string());
    }
}
