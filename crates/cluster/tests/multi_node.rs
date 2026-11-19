//! Pruebas con varios procesos: cada proceso es "otra PC" con su carpeta e identidad propias.
//! Se comunican por P2P real (QUIC por localhost, sin relays).
//!
//! `group_protocol_fake_servers`: protocolo completo con servidores simulados (rápido).
//! `real_minecraft_handover`: traspaso con servidores de Minecraft reales (lento; requiere Java 25).

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

struct Node { name: String, child: Child, stdin: ChildStdin, rx: Receiver<String>, root: std::path::PathBuf }

impl Node {
    fn spawn(name: &str, root: &std::path::Path, fake: bool, extra_env: &[(&str, &str)]) -> Node {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_939node"));
        cmd.arg("--root").arg(root).arg("--name").arg(name).env("H939_FAST", "1").env("H939_DEBUG", "1").env_remove("JAVA_TOOL_OPTIONS");
        if fake { cmd.arg("--fake"); }
        if let Ok(j) = std::env::var("HOST939_TEST_JAVA25") { cmd.arg("--java25").arg(j); }
        for (k, v) in extra_env { cmd.env(k, v); }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        let tag = name.to_string();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.starts_with("##") { println!("    [{tag}] {}", &line[3..]); }
                if line.starts_with("@@") && tx.send(line[3..].to_string()).is_err() { break; }
            }
        });
        let mut n = Node { name: name.into(), child, stdin, rx, root: root.to_path_buf() };
        let id = n.ok("id");
        println!("  · {name} = {}", &id.as_str().unwrap()[..10]);
        n
    }

    fn cmd(&mut self, c: &str) -> Value {
        writeln!(self.stdin, "{c}").unwrap();
        self.stdin.flush().unwrap();
        let line = self.rx.recv_timeout(Duration::from_secs(600)).unwrap_or_else(|_| panic!("{}: sin respuesta a «{c}»", self.name));
        serde_json::from_str(&line).unwrap()
    }
    fn ok(&mut self, c: &str) -> Value {
        let r = self.cmd(c);
        assert!(r["ok"].as_bool().unwrap(), "{}: «{c}» falló: {}", self.name, r["err"]);
        r["v"].clone()
    }
    fn err(&mut self, c: &str) -> String {
        let r = self.cmd(c);
        assert!(!r["ok"].as_bool().unwrap(), "{}: «{c}» debía fallar", self.name);
        r["err"].as_str().unwrap().to_string()
    }
    fn status(&mut self, sid: &str) -> Value { self.ok(&format!("status {sid}")) }
    fn pid(&self) -> u32 { self.child.id() }
}

impl Drop for Node {
    fn drop(&mut self) {
        // En Linux los procesos hijos (Java) no mueren con el padre: se cierran antes. Si el nodo
        // ya terminó solo (orden «exit»), no hay nada que limpiar: antes esto imprimía
        // «kill: No such process», que era ruido de limpieza y no un error.
        #[cfg(unix)]
        if matches!(self.child.try_wait(), Ok(None)) {
            let id = self.child.id().to_string();
            let quiet = |c: &mut Command| { let _ = c.stdout(Stdio::null()).stderr(Stdio::null()).status(); };
            quiet(Command::new("pkill").args(["-CONT", "-P", &id]));
            quiet(Command::new("kill").args(["-CONT", &id]));
            quiet(Command::new("pkill").args(["-KILL", "-P", &id]));
        }
        #[cfg(windows)]
        {
            let id = self.child.id();
            let _ = std::panic::catch_unwind(|| signal(id, "CONT"));
            let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &id.to_string()]).output();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        if f() { return; }
        std::thread::sleep(Duration::from_millis(400));
    }
    panic!("tiempo agotado esperando: {what}");
}

fn slp(port: u16) -> String {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        match tokio::time::timeout(Duration::from_secs(10), host939_cluster::moving::status(c)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => format!("(error: {e})"),
            Err(_) => "(sin respuesta en 10 s)".into(),
        }
    })
}

/// Congela (STOP), reanuda (CONT) o mata (KILL) una "PC": simula una PC colgada o sin red.
#[cfg(unix)]
fn signal(pid: u32, sig: &str) { assert!(Command::new("kill").arg(format!("-{sig}")).arg(pid.to_string()).status().unwrap().success()); }
#[cfg(windows)]
fn signal(pid: u32, sig: &str) {
    let script = match sig {
        "KILL" => format!("Stop-Process -Id {pid} -Force"),
        "STOP" | "CONT" => {
            let f = if sig == "STOP" { "NtSuspendProcess" } else { "NtResumeProcess" };
            format!("Add-Type -Name N -Namespace W -MemberDefinition '[DllImport(\"ntdll.dll\")] public static extern int {f}(IntPtr h);'; $p = Get-Process -Id {pid}; if ([W.N]::{f}($p.Handle) -ne 0) {{ exit 1 }}")
        }
        other => panic!("señal {other}"),
    };
    assert!(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status().unwrap().success(), "señal {sig} a {pid}");
}

fn fresh(v: &Value) -> (u64, u64) { (v["epoch"].as_u64().unwrap_or(0), v["seq"].as_u64().unwrap_or(0)) }

#[test]
fn group_protocol_fake_servers() {
    let tmp = tempfile::tempdir().unwrap();
    println!("▶ tres PCs simuladas: A (propietaria), B y C (coanfitrionas)");
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[("H939_SCORE", "100")]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[("H939_SCORE", "150")]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[("H939_SCORE", "200")]);

    let sid = a.ok("create Zombies 25801").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t1 = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let t2 = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    assert_eq!(b.ok(&format!("join {t1}")).as_str().unwrap(), sid);
    assert_eq!(c.ok(&format!("join {t2}")).as_str().unwrap(), sid);
    b.ok(&format!("setport {sid} 25802"));
    c.ok(&format!("setport {sid} 25803"));
    // Una invitación usada no sirve dos veces.
    let mut d = Node::spawn("D", &tmp.path().join("d"), true, &[]);
    assert!(d.err(&format!("join {t1}")).contains("no es válida"));
    drop(d);
    println!("✔ invitaciones: B y C se unieron; un código usado se rechaza");

    // Todos ven a los tres integrantes.
    wait("que los tres se vean", Duration::from_secs(20), || {
        [&mut a, &mut b, &mut c].iter_mut().all(|n| n.status(&sid)["members"].as_array().unwrap().iter().filter(|m| m["online"].as_bool().unwrap()).count() == 3)
    });

    // B no puede encender en paralelo: A lo tiene encendido. (Con A apagada se lo pasaría sola:
    // ver `tierra_e_inventario_no_retroceden`.)
    a.ok(&format!("start {sid}"));
    assert!(b.err(&format!("start {sid}")).contains("hosteando"));
    a.ok(&format!("write {sid} marca.txt m1"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    println!("✔ A es anfitriona; B no puede encender en paralelo (un solo escritor)");

    wait("que B y C copien el punto de A", Duration::from_secs(30), || {
        fresh(&b.status(&sid)["myLatest"]) == fresh(&s1) && fresh(&c.status(&sid)["myLatest"]) == fresh(&s1)
    });
    println!("✔ sincronización incremental verificada: B y C tienen el último punto");

    // Misma dirección para jugar desde C: 127.0.0.1:41901 → A.
    let port_c = c.ok(&format!("route {sid} 41901")).as_u64().unwrap() as u16;
    let port_b = b.ok(&format!("route {sid} 41902")).as_u64().unwrap() as u16;
    assert!(slp(port_c).contains("hosteado por A"), "{}", slp(port_c));
    println!("✔ C entra por 127.0.0.1:{port_c} y llega al servidor de A");

    // ── Traspaso planificado A → B ──
    let to = a.ok(&format!("handover {sid} {}", b.ok("id").as_str().unwrap()));
    assert_eq!(to.as_str().unwrap(), "B");
    wait("B en línea", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} marca.txt")).as_str(), Some("m1"));
    wait("C ve a B como anfitriona", Duration::from_secs(15), || slp(port_c).contains("hosteado por B"));
    assert!(slp(port_b).contains("hosteado por B"));
    let lb = b.status(&sid)["lease"].clone();
    println!("✔ traspaso A→B: B encendió con el mundo de A (marca m1), época {}; la misma dirección ahora lleva a B", lb["epoch"]);
    assert!(a.err(&format!("start {sid}")).contains("hosteando"));

    // B juega y guarda; A y C copian.
    b.ok(&format!("write {sid} marca.txt m2"));
    let s2 = b.ok(&format!("snapshot {sid}"));
    wait("A y C copian el punto de B", Duration::from_secs(30), || {
        fresh(&a.status(&sid)["myLatest"]) == fresh(&s2) && fresh(&c.status(&sid)["myLatest"]) == fresh(&s2)
    });

    // ── Caída de B (se congela: como un corte de red) → recuperación por mayoría ──
    println!("▶ B se congela (SIGSTOP)");
    let pid_b = b.pid();
    signal(pid_b, "STOP");
    let t0 = Instant::now();
    wait("que el grupo levante el servidor en otra PC", Duration::from_secs(60), || c.ok(&format!("running {sid}")).as_bool().unwrap() || a.ok(&format!("running {sid}")).as_bool().unwrap());
    let elapsed = t0.elapsed().as_secs_f32();
    assert!(c.ok(&format!("running {sid}")).as_bool().unwrap(), "debía elegir a C (mismo estado, mejor puntaje)");
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap(), "nunca dos encendidas");
    assert_eq!(c.ok(&format!("read {sid} marca.txt")).as_str(), Some("m2"));
    wait("A llega a C por la misma dirección", Duration::from_secs(15), || slp(port_c).contains("hosteado por C"));
    println!("✔ recuperación automática en {elapsed:.1} s: C elegida por mayoría (A+C), mundo con m2");

    // ── B vuelve: ve la época mayor, se aísla y aparta su copia ──
    signal(pid_b, "CONT");
    wait("B se detiene sola", Duration::from_secs(30), || !b.ok(&format!("running {sid}")).as_bool().unwrap());
    // La copia de seguridad se toma justo después de apagar: se espera a que aparezca.
    wait("B aparta su copia", Duration::from_secs(20), || b.ok(&format!("snapshots {sid}")).as_array().unwrap().iter().any(|s| s["kind"] == "seguridad"));
    assert!(b.err(&format!("start {sid}")).contains("hosteando"));
    println!("✔ B volvió, detectó que C tomó el servidor, se detuvo y apartó su copia (no se mezclan mundos)");

    // ── Soltar ordenadamente: C apaga y suelta; A lo enciende con el último estado ──
    c.ok(&format!("write {sid} marca.txt m3"));
    let confirmed = c.ok(&format!("release {sid}"));
    assert!(confirmed.as_u64().unwrap() >= 1, "al menos otra PC debía confirmar el último punto");
    wait("A ve el permiso soltado", Duration::from_secs(15), || a.status(&sid)["lease"]["state"] == "released");
    assert!(slp(port_b).contains("Nadie lo está hosteando"));
    a.ok(&format!("start {sid}"));
    assert_eq!(a.ok(&format!("read {sid} marca.txt")).as_str(), Some("m3"));
    println!("✔ C soltó con {confirmed} confirmación(es); A lo encendió con el último estado (m3); mientras tanto la dirección mostraba un aviso claro");

    // ── Salir de la app con traspaso automático ──
    let report = a.ok("quit");
    println!("    salida de A: {report}");
    wait("otra PC tomó el servidor al salir A", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap() || c.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap());
    println!("✔ al salir A, el servidor pasó solo a otra PC encendida");
    let _ = (&a.root, &b.root);
}

/// Dos PCs: sin mayoría no hay recuperación automática (no se puede distinguir "se apagó" de
/// "se cortó la red"); "Recuperar aquí" es manual. Y nadie puede encender con un estado viejo.
#[test]
fn two_pcs_manual_recovery_and_no_stale_start() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let sid = a.ok("create Duo 25821").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25822"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} marca.txt d1"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));

    // A se congela: B NO debe tomar el servidor sola (sin mayoría posible).
    let pid_a = a.pid();
    signal(pid_a, "STOP");
    std::thread::sleep(Duration::from_secs(14));
    assert!(!b.ok(&format!("running {sid}")).as_bool().unwrap(), "con dos PCs no hay toma automática");
    assert!(b.err(&format!("start {sid}")).contains("no responde"));
    println!("✔ dos PCs: sin mayoría, B no tomó el servidor sola; ofrece «Recuperar aquí»");
    b.ok(&format!("take {sid}"));
    assert!(b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} marca.txt")).as_str(), Some("d1"));
    println!("✔ «Recuperar aquí» en B: encendió con el último estado sincronizado (d1)");
    signal(pid_a, "CONT");
    wait("A se aísla al volver", Duration::from_secs(30), || !a.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("A aparta su copia", Duration::from_secs(20), || a.ok(&format!("snapshots {sid}")).as_array().unwrap().iter().any(|s| s["kind"] == "seguridad"));
    println!("✔ A volvió, vio la época nueva, se detuvo y apartó su copia");

    // B escribe y suelta mientras A está desconectada → A no puede encender con su versión vieja.
    b.ok(&format!("write {sid} marca.txt d2"));
    signal(pid_a, "STOP");
    let n = b.ok(&format!("release {sid}"));
    assert_eq!(n.as_u64().unwrap(), 0, "nadie pudo confirmar");
    // B se desconecta (sale) y A vuelve.
    let pid_b = b.pid();
    signal(pid_b, "STOP");
    signal(pid_a, "CONT");
    signal(pid_b, "CONT");
    wait("A recibe el permiso soltado", Duration::from_secs(20), || a.status(&sid)["lease"]["state"] == "released");
    // Ahora B se va de verdad sin que A haya copiado (A congelada mientras B soltó; la copia
    // puede haber ocurrido al volver). Si A ya copió, puede encender; si no, debe negarse.
    let st = a.status(&sid);
    let mine = fresh(&st["myLatest"]);
    let req = (st["lease"]["required"][0].as_u64().unwrap(), st["lease"]["required"][1].as_u64().unwrap());
    println!("    A tiene {mine:?}, se requiere {req:?}");
    wait("A copia el último punto de B", Duration::from_secs(20), || fresh(&a.status(&sid)["myLatest"]) >= req);
    drop(b);
    a.ok(&format!("start {sid}"));
    assert_eq!(a.ok(&format!("read {sid} marca.txt")).as_str(), Some("d2"));
    println!("✔ el permiso soltado exige el último punto: A encendió recién al tenerlo (d2)");
}

/// Una PC con copia vieja no puede encender aunque el permiso esté libre: tiene que tener
/// el último punto que dejó quien lo soltó. (C solo juega: retransmite el permiso pero no tiene datos.)
#[test]
fn stale_copy_cannot_start_after_release() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[]);
    let sid = a.ok("create Viejo 25831").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25832"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    c.ok(&format!("join {t}"));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    let pid_b = b.pid();
    signal(pid_b, "STOP");
    a.ok(&format!("write {sid} marca.txt nuevo"));
    let n = a.ok(&format!("release {sid}"));
    assert_eq!(n.as_u64().unwrap(), 0);
    wait("C (jugador) ve el permiso soltado", Duration::from_secs(10), || c.status(&sid)["lease"]["state"] == "released");
    drop(a); // la PC con la última versión se apaga
    signal(pid_b, "CONT");
    wait("B recibe el permiso soltado por medio de C", Duration::from_secs(20), || b.status(&sid)["lease"]["state"] == "released");
    let e = b.err(&format!("start {sid}"));
    assert!(e.contains("última versión"), "{e}");
    println!("✔ B con copia vieja no puede encender: «{e}»");
    // C, que solo juega, ve un aviso claro en su dirección fija.
    let port = c.ok(&format!("route {sid} 41931")).as_u64().unwrap() as u16;
    assert!(slp(port).contains("Nadie lo está hosteando"));
    println!("✔ el jugador ve «Nadie lo está hosteando» en la lista de servidores");
    // Si igual se quiere seguir con lo que hay, es explícito.
    b.ok(&format!("take {sid}"));
    assert!(b.ok(&format!("running {sid}")).as_bool().unwrap());
    println!("✔ «Recuperar aquí» permite seguir con la copia vieja, solo de forma explícita");
}

/// Traspaso con servidores de Minecraft reales: el mundo (con un marcador del scoreboard)
/// sobrevive al cambio de PC.
#[test]
#[ignore]
fn real_minecraft_handover() {
    let tmp = tempfile::tempdir().unwrap();
    // Sin internet (entorno de desarrollo): se precarga el jar oficial en la caché de cada "PC"
    // (se verifica por SHA-1 igual). Con internet (CI en Windows) cada PC lo descarga de Mojang.
    let jar = std::path::Path::new("/tmp/mctest/server.jar");
    if jar.exists() {
        let sha1 = host939_core::net::sha1_file(jar).unwrap();
        for n in ["a", "b"] {
            let d = tmp.path().join(n).join("cache/jars");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::copy(jar, d.join(format!("{sha1}.jar"))).unwrap();
        }
    }
    let mut a = Node::spawn("A", &tmp.path().join("a"), false, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), false, &[]);
    let sid = a.ok("create Real939 25811").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25812"));
    let route = b.ok(&format!("route {sid} 41911")).as_u64().unwrap() as u16;
    a.ok(&format!("start {sid}"));
    println!("✔ A encendió un servidor 26.2 real");
    a.ok(&format!("cmd {sid} scoreboard objectives add traspaso dummy"));
    a.ok(&format!("cmd {sid} scoreboard players set marcador traspaso 939"));
    // Como el juego, se reintenta: la primera consulta puede llegar mientras la ruta se arma.
    let mut st = String::new();
    for i in 0..10 {
        st = slp(route);
        if st.contains("\"protocol\"") { if i > 0 { println!("  (la ruta respondió al intento {})", i + 1); } break; }
        println!("  intento {}: {st}", i + 1);
        std::thread::sleep(Duration::from_secs(1));
    }
    assert!(st.contains("\"protocol\""), "estado por la ruta de B: {st}");
    println!("✔ B ve el servidor de A por 127.0.0.1:{route}");
    let t0 = Instant::now();
    a.ok(&format!("handover {sid}"));
    wait("B en línea", Duration::from_secs(400), || b.ok(&format!("online {sid}")).as_bool().unwrap());
    println!("✔ traspaso real A→B en {:.1} s", t0.elapsed().as_secs_f32());
    b.ok(&format!("cmd {sid} scoreboard players get marcador traspaso"));
    std::thread::sleep(Duration::from_secs(2));
    let log = std::fs::read_to_string(tmp.path().join("b/Servers").join(&sid).join("server/logs/latest.log")).unwrap_or_default();
    assert!(log.contains("marcador has 939"), "el marcador no sobrevivió: {}", log.lines().rev().take(15).collect::<Vec<_>>().join("\n"));
    let st = slp(route);
    assert!(st.contains("\"name\":\"26.2\""), "la ruta debía llevar al servidor real de B: {st}");
    println!("✔ el mundo sobrevivió al traspaso (marcador = 939) y la misma dirección llega al servidor de B");
    b.ok(&format!("stop {sid}"));
}

/// Nombres visibles y solicitud manual de anfitrión, sobre el traspaso de siempre:
/// renombrar no toca la identidad; una solicitud por vez; cancelar; rechazar; aceptar con el
/// mundo intacto; un traspaso que falla deja todo en la anfitriona; jugadoras no pueden pedir.
#[test]
fn host_requests_and_pc_names() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[("H939_SCORE", "200")]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[("H939_SCORE", "150"), ("H939_TEST_FAIL_PREFETCH", "1")]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[("H939_SCORE", "60"), ("H939_RAM", "3000")]);
    let mut d = Node::spawn("D", &tmp.path().join("d"), true, &[]);
    let sid = a.ok("create Zombies 25811").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    for (n, port) in [(&mut b, 25812), (&mut c, 25813)] {
        let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
        n.ok(&format!("join {t}"));
        n.ok(&format!("setport {sid} {port}"));
    }
    let tj = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    d.ok(&format!("join {tj}"));
    wait("que se vean los cuatro", Duration::from_secs(20), || a.status(&sid)["members"].as_array().unwrap().iter().filter(|m| m["online"].as_bool().unwrap()).count() == 4);
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} marca.txt m1"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B y C copian", Duration::from_secs(30), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1) && fresh(&c.status(&sid)["myLatest"]) == fresh(&s1));

    // ── Nombre visible: solo visual ──
    let id_b = b.ok("id");
    let lease_before = a.status(&sid)["lease"].clone();
    b.ok("rename Pampa faro");
    wait("A ve el nombre nuevo de B", Duration::from_secs(15), || {
        a.status(&sid)["members"].as_array().unwrap().iter().any(|m| m["nodeId"] == id_b && m["name"] == "Pampa" && m["symbol"] == "faro")
    });
    assert_eq!(b.ok("id"), id_b, "el nombre no cambia la identidad");
    assert_eq!(a.status(&sid)["lease"], lease_before, "el nombre no toca el permiso");
    println!("✔ B se llama «Pampa» (símbolo faro) para todo el grupo; su identidad y el permiso no cambiaron");

    // ── Una jugadora no puede pedir; una coanfitriona sí ──
    assert!(d.err(&format!("request {sid}")).contains("jugadora"));
    assert_eq!(c.ok(&format!("request {sid}")).as_str(), Some("A"));
    let inc = a.status(&sid)["incomingRequest"].clone();
    assert_eq!(inc["name"], "C");
    assert!(inc["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("memoria")), "advertencia de memoria: {inc}");
    assert!(inc["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("procesador")), "advertencia de CPU: {inc}");
    assert!(c.status(&sid)["outgoingRequest"].is_object());
    println!("✔ C pidió alojarlo; A ve la solicitud con advertencias: {}", inc["warnings"]);

    // ── Sin duplicados ni simultáneas ──
    assert!(c.err(&format!("request {sid}")).contains("Ya le pediste"));
    assert!(b.err(&format!("request {sid}")).contains("Ya hay una solicitud pendiente de C"));
    println!("✔ solicitudes duplicadas o simultáneas rechazadas con un mensaje claro");

    // ── Cancelar ──
    c.ok(&format!("cancelreq {sid}"));
    wait("A ya no ve la solicitud", Duration::from_secs(5), || a.status(&sid)["incomingRequest"].is_null());
    println!("✔ C canceló su solicitud");

    // ── Rechazar ──
    b.ok(&format!("request {sid}"));
    wait("A ve la solicitud de Pampa", Duration::from_secs(5), || a.status(&sid)["incomingRequest"]["name"] == "Pampa");
    a.ok(&format!("answer {sid} no"));
    wait("Pampa recibe el rechazo", Duration::from_secs(5), || { let st = b.status(&sid); st["lastAnswer"]["accepted"] == false && st["outgoingRequest"].is_null() });
    assert!(a.ok(&format!("running {sid}")).as_bool().unwrap());
    println!("✔ A rechazó; Pampa recibió la respuesta y A siguió alojando");

    // ── Aceptar: traspaso de siempre, mundo intacto, un solo escritor ──
    c.ok(&format!("request {sid}"));
    wait("A ve la solicitud de C", Duration::from_secs(5), || a.status(&sid)["incomingRequest"]["name"] == "C");
    assert_eq!(a.ok(&format!("answer {sid} si")).as_str(), Some("C"));
    wait("C en línea", Duration::from_secs(30), || c.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap(), "nunca dos encendidas");
    assert_eq!(c.ok(&format!("read {sid} marca.txt")).as_str(), Some("m1"));
    wait("C sin solicitud pendiente", Duration::from_secs(10), || c.status(&sid)["outgoingRequest"].is_null());
    println!("✔ A aceptó: C es la anfitriona con el mundo de A (m1); A está apagada");

    // ── Traspaso que falla: la anfitriona conserva el control ──
    c.ok(&format!("write {sid} marca.txt m2"));
    b.ok(&format!("request {sid}"));
    wait("C ve la solicitud de Pampa", Duration::from_secs(5), || c.status(&sid)["incomingRequest"]["name"] == "Pampa");
    let e = c.err(&format!("answer {sid} si"));
    assert!(e.contains("sigue en esta PC") || e.contains("siguió en esta PC"), "{e}");
    assert!(c.ok(&format!("running {sid}")).as_bool().unwrap(), "C sigue encendida");
    assert!(!b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(c.status(&sid)["iAmHost"].as_bool().unwrap());
    assert_eq!(c.ok(&format!("read {sid} marca.txt")).as_str(), Some("m2"));
    wait("Pampa se entera de la falla", Duration::from_secs(5), || b.status(&sid)["lastAnswer"]["accepted"] == false);
    println!("✔ el traspaso a Pampa falló al copiar: C siguió alojando sin cortar, con el mundo intacto (m2)");

    // ── Anfitriona que no responde: no se puede pedir (actúa la recuperación) ──
    let pid_c = c.pid();
    signal(pid_c, "STOP");
    std::thread::sleep(Duration::from_secs(3));
    let e = b.err(&format!("request {sid}"));
    assert!(e.contains("no responde"), "{e}");
    signal(pid_c, "CONT");
    println!("✔ con la anfitriona sin responder no se pide: queda la recuperación automática");
}

/// Traspaso aceptado con una copia lenta: mientras la otra PC copia (fase 1), el servidor sigue
/// encendido en la anfitriona y los jugadores siguen entrando; nunca hay dos encendidos.
#[test]
fn slow_two_phase_handover_keeps_playing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[("H939_TEST_SLOW_PREFETCH", "4")]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[]);
    let sid = a.ok("create Lento 25821").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    for (n, port) in [(&mut b, 25822), (&mut c, 25823)] {
        let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
        n.ok(&format!("join {t}"));
        n.ok(&format!("setport {sid} {port}"));
    }
    wait("que se vean", Duration::from_secs(20), || a.status(&sid)["members"].as_array().unwrap().iter().filter(|m| m["online"].as_bool().unwrap()).count() == 3);
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} marca.txt lento"));
    let port_c = c.ok(&format!("route {sid} 41931")).as_u64().unwrap() as u16;
    wait("C llega a A", Duration::from_secs(15), || slp(port_c).contains("hosteado por A"));
    b.ok(&format!("request {sid}"));
    wait("A ve la solicitud", Duration::from_secs(5), || a.status(&sid)["incomingRequest"]["name"] == "B");
    // A acepta (la orden queda esperando el traspaso); mientras tanto se mira desde C y B.
    writeln!(a.stdin, "answer {sid} si").unwrap();
    a.stdin.flush().unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    assert!(slp(port_c).contains("hosteado por A"), "durante la fase 1 se sigue jugando en A: {}", slp(port_c));
    assert!(!b.ok(&format!("running {sid}")).as_bool().unwrap(), "B no enciende antes de tener el permiso");
    let line = a.rx.recv_timeout(Duration::from_secs(120)).expect("respuesta de A");
    let r: Value = serde_json::from_str(&line).unwrap();
    assert!(r["ok"].as_bool().unwrap(), "traspaso: {r}");
    wait("B en línea", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} marca.txt")).as_str(), Some("lento"));
    wait("C llega a B", Duration::from_secs(15), || slp(port_c).contains("hosteado por B"));
    println!("✔ copia lenta: se siguió jugando en A durante la fase 1; después B tomó el servidor con el mundo intacto y la misma dirección llevó a B");
}

/// Regresión (CI 15/11/2026): después de un traspaso A→C, B pedía alojar y recibía «C no
/// responde» aunque C estaba bien. Causa: B consultaba a todas las PCs juntas y solo marcaba la
/// respuesta de C cuando terminaba también la de A, que respondía lento (ocupada apagando). Acá A
/// responde siempre con 3,5 s de demora: B tiene que poder pedir a C una y otra vez.
#[test]
fn request_reaches_new_host_while_old_host_is_slow() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[("H939_TEST_SLOW_INFO_MS", "3500")]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[]);
    let sid = a.ok("create Lento 25881").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t1 = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let t2 = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t1}"));
    c.ok(&format!("join {t2}"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("snapshot {sid} prueba"));
    wait("B y C copian", Duration::from_secs(40), || b.status(&sid)["myLatest"].is_object() && c.status(&sid)["myLatest"].is_object());
    // C pide alojar (se reintenta hasta que C conoce el permiso de A).
    wait("C pide alojar", Duration::from_secs(30), || c.cmd(&format!("request {sid}"))["ok"] == true);
    wait("A ve la solicitud de C", Duration::from_secs(15), || a.status(&sid)["incomingRequest"]["name"] == "C");
    a.ok(&format!("answer {sid} si"));
    wait("C aloja", Duration::from_secs(60), || c.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("B sabe que C aloja", Duration::from_secs(20), || b.status(&sid)["lease"]["hostName"] == "C");
    println!("✔ traspaso A→C hecho; A sigue respondiendo con 3,5 s de demora");
    for i in 1..=6 {
        let r = b.cmd(&format!("request {sid}"));
        assert_eq!(r["ok"], true, "intento {i}: B no pudo pedir a C: {r}");
        b.ok(&format!("cancelreq {sid}"));
        std::thread::sleep(Duration::from_millis(900));
    }
    println!("✔ B pidió alojar a C 6 veces seguidas sin «C no responde», con A lenta");
    let online = b.status(&sid)["members"].as_array().unwrap().iter().find(|m| m["name"] == "C").map(|m| m["online"] == true).unwrap_or(false);
    assert!(online, "B debe ver a C conectada");
}

/// Administración remota de archivos entre PCs del grupo: listar, crear, subir y bajar con
/// verificación por SHA-1, sin sobrescribir sin pedirlo, mundo protegido con el servidor
/// encendido, cancelación sin archivos a medias, permisos por rol y que siga a la anfitriona.
#[test]
fn remote_files_follow_host_and_protect_world() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[]);
    let sid = a.ok("create Archivos 25841").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25842"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    c.ok(&format!("join {t}"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} marca.txt hola"));
    wait("B ve a A como anfitriona", Duration::from_secs(20), || b.status(&sid)["lease"]["state"] == "hosting");

    // Listar y crear carpeta desde B: pasa en A.
    let ls = b.ok(&format!("fls {sid}"));
    assert!(ls.as_array().unwrap().iter().any(|e| e["name"] == "world" && e["locked"] == true), "{ls}");
    b.ok(&format!("fmkdir {sid} config/remoto"));
    assert!(a.ok(&format!("exists {sid} config/remoto")).as_bool().unwrap());
    println!("✔ B lista y crea carpetas en la anfitriona A; el mundo figura protegido (encendido)");

    // Subir 3 MB y bajarlos: idénticos.
    let big = tmp.path().join("grande.bin");
    let mut data = vec![0u8; 3 * 1024 * 1024 + 123];
    for (i, x) in data.iter_mut().enumerate() { *x = (i * 7 % 251) as u8; }
    std::fs::write(&big, &data).unwrap();
    let local = host939_core::net::sha1_file(&big).unwrap();
    b.ok(&format!("fput {sid} {} config/remoto/grande.bin", big.display()));
    assert_eq!(a.ok(&format!("sha1 {sid} config/remoto/grande.bin")).as_str().unwrap(), local);
    let back = tmp.path().join("bajado.bin");
    b.ok(&format!("fget {sid} config/remoto/grande.bin {}", back.display()));
    assert_eq!(host939_core::net::sha1_file(&back).unwrap(), local);
    println!("✔ subida y descarga de 3 MB por P2P con el mismo SHA-1 en las dos puntas");

    // Sin reemplazo explícito no se pisa; con reemplazo sí.
    let e = b.err(&format!("fput {sid} {} config/remoto/grande.bin", big.display()));
    assert!(e.contains("exists"), "{e}");
    b.ok(&format!("fput {sid} {} config/remoto/grande.bin reemplazar", big.display()));
    println!("✔ no sobrescribe sin pedirlo («{e}»)");

    // Cancelar a mitad de camino: no queda nada a medias.
    let e = b.err(&format!("fputcancel {sid} {} config/remoto/cortado.bin", big.display()));
    assert!(e.contains("cancelada"), "{e}");
    std::thread::sleep(Duration::from_millis(800));
    let names = a.ok(&format!("lsdir {sid} config/remoto"));
    assert!(!names.as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("cortado") || n.as_str().unwrap().contains(".939-subida")), "{names}");
    println!("✔ subida cancelada: ni el archivo ni el temporal quedan en la anfitriona");

    // Mundo protegido mientras está encendido.
    let e = b.err(&format!("frm {sid} world/marca.txt"));
    assert!(e.contains("encendido"), "{e}");
    let e = b.err(&format!("fget {sid} world/marca.txt {}", tmp.path().join("m.txt").display()));
    assert!(e.contains("encendido"), "{e}");
    println!("✔ con el servidor encendido el mundo no se modifica ni se descarga");

    // Una PC que solo juega no administra archivos.
    let e = c.err(&format!("fls {sid}"));
    assert!(e.contains("propietaria y las coanfitrionas"), "{e}");
    println!("✔ la PC jugadora no puede administrar archivos");

    // Apagado: ahora sí se puede tocar el mundo (lo borrado va a la Papelera de A).
    a.ok(&format!("stop {sid}"));
    wait("A apagado", Duration::from_secs(20), || !a.ok(&format!("running {sid}")).as_bool().unwrap());
    // Al detener, A guarda el estado final en segundo plano; mientras tanto no acepta otro punto
    // («Ya hay una operación en curso»). Se espera a que termine (en Windows tarda más).
    wait("A guarda el estado final", Duration::from_secs(30), || a.ok(&format!("latest {sid}"))["label"] == "Al detener el servidor");
    b.ok(&format!("frm {sid} world/marca.txt"));
    assert!(!a.ok(&format!("exists {sid} world/marca.txt")).as_bool().unwrap());
    println!("✔ apagado: B manda un archivo del mundo a la Papelera de A");

    // Traspaso a B: ahora A administra los archivos de B.
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    a.ok(&format!("handover {sid} {}", b.ok("id").as_str().unwrap()));
    wait("B aloja", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("A ve a B como anfitriona", Duration::from_secs(20), || a.status(&sid)["lease"]["host"] == b.ok("id"));
    a.ok(&format!("fmkdir {sid} config/desde-a"));
    assert!(b.ok(&format!("exists {sid} config/desde-a")).as_bool().unwrap());
    println!("✔ tras el traspaso, los archivos se administran en la nueva anfitriona B");
}

/// El logo del servidor viaja firmado en sus datos públicos: lo ve la PC que solo juega y se
/// mantiene cuando cambia la anfitriona.
#[test]
fn server_logo_reaches_players_and_survives_handover() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[]);
    let sid = a.ok("create Logos 25851").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25852"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    c.ok(&format!("join {t}"));
    assert!(c.ok(&format!("card {sid}"))["logo"].is_null());
    assert_eq!(a.ok(&format!("logo {sid}")).as_bool(), Some(true));
    wait("C recibe el logo", Duration::from_secs(20), || c.ok(&format!("card {sid}"))["logo"].as_str().map(|l| l.starts_with("data:image/png;base64,")).unwrap_or(false));
    println!("✔ la PC que solo juega recibe el logo firmado por la propietaria");
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    a.ok(&format!("handover {sid} {}", b.ok("id").as_str().unwrap()));
    wait("B aloja", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    b.ok(&format!("sync {sid}")); c.ok(&format!("sync {sid}"));
    std::thread::sleep(Duration::from_secs(2));
    assert!(c.ok(&format!("card {sid}"))["logo"].as_str().unwrap_or("").starts_with("data:image/png"), "el logo se perdió tras el traspaso");
    assert!(b.ok(&format!("card {sid}"))["logo"].as_str().unwrap_or("").starts_with("data:image/png"));
    println!("✔ tras pasar el servidor a B, el logo sigue en todas las PCs");
}


/// Regresión del bloque de tierra (prueba entre bout y Archi): una PC colocó tierra, detuvo el
/// servidor y la otra lo encendió con un estado viejo (la tierra desapareció y volvió al
/// inventario). Causas que cubre:
/// 1. Un punto tomado en una PC que NO aloja (botón «Crear punto», copia vieja en disco) recibía la
///    época actual y un número de orden mayor que el último de la anfitriona: al alojar, esa PC
///    levantaba su copia vieja.
/// 2. «Detener» no publicaba el estado final: quedaba solo en la PC anfitriona.
/// 3. Encender en otra PC con la anfitriona conectada y apagada exigía un traspaso manual.
#[test]
fn tierra_e_inventario_no_retroceden() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let sid = a.ok("create Tierra 25841").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25842"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} bloque.txt pasto"));
    a.ok(&format!("write {sid} inventario.txt 1-tierra"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia el primer punto", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));

    // B tiene en disco una copia vieja y alguien crea un punto ahí (como el botón «Crear punto»).
    b.ok(&format!("writestale {sid} bloque.txt pasto"));
    b.ok(&format!("writestale {sid} inventario.txt 1-tierra"));
    let local = b.ok(&format!("manualsnap {sid}"));
    assert_eq!(local["kind"], "local", "un punto en una PC que no aloja queda como copia local");
    assert_eq!(fresh(&b.ok(&format!("latest {sid}"))), fresh(&s1), "la copia local no pasa por «lo más reciente»");
    println!("✔ el punto de una PC que no aloja queda como copia local y no compite con la anfitriona");

    // En A se coloca la tierra (sale del inventario) y se detiene el servidor con «Detener».
    a.ok(&format!("write {sid} bloque.txt tierra"));
    a.ok(&format!("write {sid} inventario.txt vacio"));
    a.ok(&format!("stop {sid}"));
    wait("B recibe el estado final sin traspaso", Duration::from_secs(25), || fresh(&b.status(&sid)["myLatest"]) > fresh(&s1));
    println!("✔ «Detener» publicó el estado final y B lo copió");

    // B enciende: A está conectada y apagada, así que se lo pasa sola.
    let e = b.err(&format!("start {sid}"));
    assert!(e.contains("te lo está pasando"), "{e}");
    wait("B queda alojando", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} bloque.txt")).as_str(), Some("tierra"), "la tierra sigue colocada");
    assert_eq!(b.ok(&format!("read {sid} inventario.txt")).as_str(), Some("vacio"), "el inventario no vuelve atrás");
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap());
    println!("✔ B encendió con la tierra colocada y el inventario correcto");

    // B juega, detiene y su PC se apaga de golpe: A recupera con lo último (publicado al detener).
    b.ok(&format!("write {sid} bloque.txt piedra"));
    b.ok(&format!("stop {sid}"));
    wait("A recibe el estado final de B", Duration::from_secs(25), || a.ok(&format!("latest {sid}"))["origin"].as_str() == Some(&id_of(&mut b)));
    let pid_b = b.pid();
    signal(pid_b, "KILL");
    wait("A puede recuperar (B dejó de responder)", Duration::from_secs(90), || a.cmd(&format!("take {sid}"))["ok"].as_bool().unwrap());
    assert_eq!(a.ok(&format!("read {sid} bloque.txt")).as_str(), Some("piedra"), "lo hecho antes de detener no se pierde aunque la PC se apague");
    println!("✔ con la PC anfitriona apagada de golpe después de detener, no se perdió nada");
}

/// 939 Host muere de golpe con un servidor compartido encendido. En Windows, el guardián le manda
/// «stop» a Minecraft (que guarda) y deja la marca «pendiente-publicar». Al volver a abrir 939 Host,
/// esta PC (que sigue con el permiso) publica ese estado final al grupo: nada de lo jugado después
/// del último punto se pierde ni queda solo en esta PC. Acá se reproduce el efecto del guardián.
#[test]
fn tras_un_cierre_inesperado_se_publica_lo_que_guardo_el_guardian() {
    let tmp = tempfile::tempdir().unwrap();
    let root_a = tmp.path().join("a");
    let mut a = Node::spawn("A", &root_a, true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let sid = a.ok("create Guardian 25861").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25862"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} bloque.txt pasto"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia el punto", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    // Se juega después del último punto y 939 Host muere de golpe.
    a.ok(&format!("write {sid} bloque.txt tierra"));
    signal(a.pid(), "KILL");
    drop(a);
    // Lo que hace el guardián: Minecraft guardó (el mundo ya está en disco) y queda la marca.
    let dir = root_a.join("Servers").join(&sid);
    assert!(dir.join("server/world/bloque.txt").exists(), "el mundo de A está en disco");
    std::fs::write(dir.join("pendiente-publicar"), b"1").unwrap();
    let _ = std::fs::remove_file(dir.join("en-marcha"));
    // Se vuelve a abrir 939 Host en A: publica el estado final sin encender nada.
    let mut a = Node::spawn("A", &root_a, true, &[]);
    wait("B recibe el estado que guardó el guardián", Duration::from_secs(30), || fresh(&b.status(&sid)["myLatest"]) > fresh(&s1));
    assert!(!dir.join("pendiente-publicar").exists(), "la marca se usó una vez");
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap(), "no se enciende solo");
    // B aloja después: tiene lo jugado antes del cierre inesperado.
    let e = b.err(&format!("start {sid}"));
    assert!(e.contains("te lo está pasando"), "{e}");
    wait("B queda alojando", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} bloque.txt")).as_str(), Some("tierra"), "lo jugado antes del cierre inesperado no se pierde");
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap(), "nunca dos anfitrionas");
    println!("✔ tras un cierre inesperado, el estado que guardó el guardián llega al grupo y nadie aloja dos veces");
}

/// Corte de luz (o «Finalizar árbol de procesos»): muere 939 Host y nadie deja la marca del guardián.
/// Lo que quedó en disco después del último punto no se publica como si fuera seguro; el grupo sigue
/// desde el último punto, y lo de la PC cortada no se borra: queda como punto apartado.
#[test]
fn tras_un_corte_sin_guardian_no_se_publica_lo_no_confirmado() {
    let tmp = tempfile::tempdir().unwrap();
    let root_a = tmp.path().join("a");
    let mut a = Node::spawn("A", &root_a, true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let sid = a.ok("create Corte 25871").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25872"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("write {sid} bloque.txt pasto"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia el punto", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    a.ok(&format!("write {sid} bloque.txt tierra"));
    signal(a.pid(), "KILL");
    drop(a);
    let mut a = Node::spawn("A", &root_a, true, &[]);
    // Al abrir, A avisa una vez que el último cierre no lo vio nadie.
    let notes = a.ok("cierres").to_string();
    assert!(notes.contains("cierre-sin-ver") && notes.contains("sin-confirmar"), "sin aviso del cierre sin ver: {notes}");
    assert_eq!(a.ok("cierres"), serde_json::json!([]), "el aviso se muestra una sola vez");
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(fresh(&b.status(&sid)["myLatest"]), fresh(&s1), "se publicó solo un estado que nadie confirmó como guardado");
    println!("    tras volver a abrir A: aviso de cierre sin ver y el grupo sigue en el último punto");
    // B pide alojar: A le pasa lo que quedó en disco (es lo más nuevo), rotulado como sin confirmar,
    // y el punto anterior sigue disponible para volver.
    let e = b.err(&format!("start {sid}"));
    assert!(e.contains("te lo está pasando"), "{e}");
    wait("B queda alojando", Duration::from_secs(60), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    assert_eq!(b.ok(&format!("read {sid} bloque.txt")).as_str(), Some("tierra"));
    let snaps = b.ok(&format!("snapshots {sid}")).to_string();
    assert!(snaps.contains("tras un cierre sin confirmar"), "el punto del traspaso no está rotulado: {snaps}");
    assert!(snaps.contains(s1["id"].as_str().expect("id del punto")), "falta el punto anterior para volver: {snaps}");
    assert!(!a.ok(&format!("running {sid}")).as_bool().unwrap(), "nunca dos anfitrionas");
    assert_eq!(a.ok("cierres"), serde_json::json!([]), "la marca se usó en el traspaso");
    println!("✔ corte sin guardián: aviso, nada se publica solo, el traspaso queda rotulado y el punto anterior sigue");
}

fn id_of(n: &mut Node) -> String { n.ok("id").as_str().unwrap().to_string() }

/// Chat de voz (Simple Voice Chat usa UDP en otro puerto): llega a la anfitriona por la conexión
/// cifrada del grupo y vuelve, sin abrir puertos. Una PC fuera del grupo no puede usarlo.
#[test]
fn voz_por_udp_llega_a_la_anfitriona() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let sid = a.ok("create Voz 25851").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    a.ok(&format!("start {sid}"));
    a.ok(&format!("fakevoice {sid} 24854"));
    b.ok(&format!("voiceroute {sid} 41854"));
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut got = String::new();
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end && got.is_empty() {
        sock.send_to(b"hola-voz", "127.0.0.1:41854").unwrap();
        let mut buf = [0u8; 256];
        if let Ok((n, _)) = sock.recv_from(&mut buf) { got = String::from_utf8_lossy(&buf[..n]).to_string(); }
    }
    assert_eq!(got, "eco:hola-voz", "la voz llega a la anfitriona y vuelve por el túnel");
    println!("✔ UDP de voz: jugadora → anfitriona → jugadora por la conexión cifrada");

    // Una PC que no es del grupo no puede usar el túnel de voz de A.
    let mut x = Node::spawn("X", &tmp.path().join("x"), true, &[]);
    let addr_a = a.ok("myaddr");
    x.ok(&format!("knowaddr {addr_a}"));
    // Para X el servidor no existe: no tiene permiso ni sabe quién aloja; no puede abrir la ruta.
    let r = x.cmd(&format!("voiceroute {sid} 41855"));
    if r["ok"].as_bool().unwrap() {
        let s2 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        s2.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s2.send_to(b"intruso", "127.0.0.1:41855").unwrap();
        let mut buf = [0u8; 64];
        assert!(s2.recv_from(&mut buf).is_err(), "una PC ajena no recibe respuesta");
    }
    println!("✔ una PC fuera del grupo no puede usar la voz");
}

/// Medición repetible del consumo en reposo de tres PCs (grupo de tres y amigos entre todas), con
/// los tiempos reales del producto (no los acelerados de las pruebas). Solo Linux (lee /proc).
/// Se corre a mano: `cargo test -p host939-cluster --features pruebas --test multi_node
/// consumo_en_reposo -- --ignored --nocapture` (con RUST_BACKTRACE=0: si está activo, cada error
/// esperable captura un backtrace y la medición sale inflada).
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn consumo_en_reposo() {
    fn cpu_ticks(pid: u32) -> u64 {
        let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let f: Vec<&str> = s.rsplit(')').next().unwrap().split_whitespace().collect();
        f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap()
    }
    fn rss_mb(pid: u32) -> u64 {
        let s = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        s.lines().find(|l| l.starts_with("VmRSS")).unwrap().split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024
    }
    // Bytes que el proceso escribió a disco (incluye lo que queda en caché del sistema).
    fn written(pid: u32) -> u64 {
        std::fs::read_to_string(format!("/proc/{pid}/io")).ok().and_then(|s| s.lines().find(|l| l.starts_with("wchar:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok())).unwrap_or(0)
    }
    fn medir(nombre: &str, nodos: &[(&str, u32)], segs: u64) {
        let t0: Vec<u64> = nodos.iter().map(|(_, p)| cpu_ticks(*p)).collect();
        let w0: Vec<u64> = nodos.iter().map(|(_, p)| written(*p)).collect();
        std::thread::sleep(Duration::from_secs(segs));
        for (i, (n, p)) in nodos.iter().enumerate() {
            let d = cpu_ticks(*p) - t0[i];
            let w = written(*p).saturating_sub(w0[i]);
            println!("MEDICION {nombre} {n}: CPU {:.2} % de un núcleo ({d} ticks en {segs} s), RSS {} MB, escrito {:.1} KB/min", d as f64 / segs as f64, rss_mb(*p), w as f64 / 1024.0 / (segs as f64 / 60.0));
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let env = [("H939_REAL_TIMING", "1")];
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &env);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &env);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &env);
    let ps = [("A", a.pid()), ("B", b.pid()), ("C", c.pid())];
    medir("solo", &ps, 60);
    let sid = a.ok("create Consumo 25971").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t1 = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let t2 = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    b.ok(&format!("join {t1}"));
    c.ok(&format!("join {t2}"));
    for (x, y) in [(0usize, 1usize), (0, 2), (1, 2)] {
        let code = [&mut a, &mut b, &mut c][x].ok("finvite").as_str().unwrap().to_string();
        let _ = [&mut a, &mut b, &mut c][y].cmd(&format!("fadd {code}"));
    }
    std::thread::sleep(Duration::from_secs(20));
    medir("grupo y amigos, apagado", &ps, 120);
    a.ok(&format!("start {sid}"));
    std::thread::sleep(Duration::from_secs(10));
    medir("grupo y amigos, servidor encendido", &ps, 120);
}
