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
        // En Linux los procesos hijos (Java) no mueren con el padre: se cierran antes.
        let _ = Command::new("pkill").arg("-CONT").arg("-P").arg(self.child.id().to_string()).status();
        let _ = Command::new("kill").arg("-CONT").arg(self.child.id().to_string()).status();
        let _ = Command::new("pkill").arg("-KILL").arg("-P").arg(self.child.id().to_string()).status();
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
        tokio::time::timeout(Duration::from_secs(10), host939_cluster::moving::status(c)).await.unwrap().unwrap_or_default()
    })
}

fn signal(pid: u32, sig: &str) { assert!(Command::new("kill").arg(format!("-{sig}")).arg(pid.to_string()).status().unwrap().success()); }

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

    // B no puede encender: el permiso es de A.
    assert!(b.err(&format!("start {sid}")).contains("hosteando"));
    a.ok(&format!("start {sid}"));
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
    let snaps = b.ok(&format!("snapshots {sid}"));
    assert!(snaps.as_array().unwrap().iter().any(|s| s["kind"] == "seguridad"), "B debe apartar su copia: {snaps}");
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
    assert!(a.ok(&format!("snapshots {sid}")).as_array().unwrap().iter().any(|s| s["kind"] == "seguridad"));
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
    let jar = std::path::Path::new("/tmp/mctest/server.jar");
    assert!(jar.exists(), "falta /tmp/mctest/server.jar");
    std::env::var("HOST939_TEST_JAVA25").expect("definí HOST939_TEST_JAVA25");
    let tmp = tempfile::tempdir().unwrap();
    // Precargar el jar oficial en la caché de cada "PC" (se verifica por SHA-1 igual).
    let sha1 = String::from_utf8(Command::new("sha1sum").arg(jar).output().unwrap().stdout).unwrap().split_whitespace().next().unwrap().to_string();
    for n in ["a", "b"] {
        let d = tmp.path().join(n).join("cache/jars");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::copy(jar, d.join(format!("{sha1}.jar"))).unwrap();
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
    let st = slp(route);
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
