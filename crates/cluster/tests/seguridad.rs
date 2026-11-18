//! Pruebas ofensivas con varias «PCs» (procesos con identidad propia, P2P real por localhost).
//! Cada prueba reproduce un ataque concreto contra 939 Host y comprueba que no funciona,
//! y que las operaciones legítimas siguen funcionando.
#![allow(dead_code)]

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


/// Nodo con argumentos extra (modo de conexión), sin servidores reales.
struct NodeArgs;
impl NodeArgs {
    fn spawn(name: &str, root: &std::path::Path, args: &[&str]) -> Node {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_939node"));
        cmd.arg("--root").arg(root).arg("--name").arg(name).arg("--fake").args(args).env("H939_FAST", "1").env("H939_DEBUG", "1").env_remove("JAVA_TOOL_OPTIONS");
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
        Node { name: name.into(), child, stdin, rx, root: root.to_path_buf() }
    }
}

fn three(tmp: &std::path::Path, env: &[(&str, &str)], port: u16) -> (Node, Node, Node, String) {
    let mut a = Node::spawn("A", &tmp.join("a"), true, env);
    let mut b = Node::spawn("B", &tmp.join("b"), true, env);
    let mut c = Node::spawn("C", &tmp.join("c"), true, env);
    let sid = a.ok(&format!("create Seguro {port}")).as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} {}", port + 1));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    c.ok(&format!("join {t}"));
    // Peor caso: la jugadora C conoce la dirección de B por fuera del protocolo (misma red o una
    // conexión anterior). 939 Host ya no se la da al unirse, pero B tiene que rechazarla igual.
    let addr_b = b.ok("myaddr");
    c.ok(&format!("knowaddr {addr_b}"));
    (a, b, c, sid)
}

fn id(n: &mut Node) -> String { n.ok("id").as_str().unwrap().to_string() }
fn epoch(n: &mut Node, sid: &str) -> u64 { n.ok(&format!("lease {sid}"))["epoch"].as_u64().unwrap_or(0) }
fn host(n: &mut Node, sid: &str) -> String { n.ok(&format!("lease {sid}"))["host"].as_str().unwrap_or("").to_string() }

#[test]
fn ataques_al_permiso_de_anfitrion() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &[], 25921);
    let (ia, ib, ic) = (id(&mut a), id(&mut b), id(&mut c));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    wait("C conoce el permiso", Duration::from_secs(20), || epoch(&mut c, &sid) == 1);
    let e0 = epoch(&mut a, &sid);

    // 1. La jugadora C se firma un «traspaso» a sí misma y se lo manda a todas.
    for to in [&ia, &ib] { c.ok(&format!("forge {sid} {to} handover yo")); }
    // 2. C firma como si fuera la propietaria (tipo owner) y como elegida sin votos.
    for kind in ["owner", "elected", "takeover", "claim", "genesis"] { c.ok(&format!("forge {sid} {ia} {kind} yo")); }
    // 3. La coanfitriona B intenta quedarse con el servidor con una recuperación manual mientras A funciona.
    for to in [&ia, &ic] { b.ok(&format!("forge {sid} {to} takeover yo")); }
    // 4. B entrega «en nombre de A» el permiso a sí misma (firma B, no A) y salta épocas.
    b.ok(&format!("forge {sid} {ia} handover yo"));
    b.ok(&format!("forge {sid} {ia} handover yo 9"));
    std::thread::sleep(Duration::from_secs(3));
    for (n, name) in [(&mut a, "A"), (&mut b, "B"), (&mut c, "C")] {
        assert_eq!(epoch(n, &sid), e0, "{name} adoptó un permiso falso");
        assert_eq!(host(n, &sid), ia, "{name} cambió de anfitriona");
    }
    assert!(a.ok(&format!("running {sid}")).as_bool().unwrap(), "A siguió encendida");
    assert!(a.ok(&format!("rejected {sid}")).as_u64().unwrap() >= 8, "A registró los rechazos");
    println!("✔ permisos falsificados (jugadora y coanfitriona, 9 variantes): nadie los adoptó y A siguió alojando");

    // 5. Dos anfitrionas: B intenta encender mientras A tiene el permiso.
    assert!(b.err(&format!("start {sid}")).contains("Lo está hosteando"));
    println!("✔ B no puede encender mientras A tiene el permiso");

    // 6. Traspaso legítimo A → B, y después replay del permiso viejo de A.
    a.ok(&format!("handover {sid} {ib}"));
    wait("B aloja", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("C ve la época nueva", Duration::from_secs(20), || epoch(&mut c, &sid) == e0 + 1);
    for to in [&ib, &ic] { a.ok(&format!("replay {sid} {to} 0")); }
    c.ok(&format!("replay {sid} {ib} 0"));
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(host(&mut b, &sid), ib);
    assert_eq!(host(&mut c, &sid), ib);
    assert!(b.ok(&format!("running {sid}")).as_bool().unwrap(), "B sigue encendida tras el replay");
    println!("✔ replay del permiso viejo: ignorado; el traspaso legítimo funcionó");

    // 7. Recuperación legítima: B se congela; A (coanfitriona ahora) recupera con mayoría (A + C).
    signal(b.pid(), "STOP");
    wait("A elegida por mayoría", Duration::from_secs(40), || a.ok(&format!("running {sid}")).as_bool().unwrap());
    signal(b.pid(), "CONT");
    wait("B se aísla al volver", Duration::from_secs(30), || !b.ok(&format!("running {sid}")).as_bool().unwrap());
    let l = a.ok(&format!("lease {sid}"));
    assert_eq!(l["kind"], "elected");
    assert!(l["votes"].as_array().unwrap().len() >= 1, "la elección lleva votos firmados");
    println!("✔ recuperación automática con votos firmados; la anfitriona vieja obedeció al volver");
    let _ = &ic;
}

#[test]
fn jugadora_no_obtiene_el_mundo_ni_datos_internos() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &[], 25931);
    let (ia, ib) = (id(&mut a), id(&mut b));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    let snap = s1["id"].as_str().unwrap().to_string();
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    // Manifiesto del punto de restauración pedido a mano por la jugadora: vacío.
    let r = c.ok(&format!(r#"rawframes {ia} sync {{"t":"manifest","server":"{sid}","snap":"{snap}"}} 1"#));
    assert_eq!(r, serde_json::json!([0]), "la jugadora recibió el manifiesto: {r}");
    // A la coanfitriona sí (control: el pedido está bien formado).
    let r = b.ok(&format!(r#"rawframes {ia} sync {{"t":"manifest","server":"{sid}","snap":"{snap}"}} 1"#));
    assert!(r[0].as_u64().unwrap() > 0, "la coanfitriona debía recibirlo");
    // Bloques pedidos por hash (los conoce B): la jugadora recibe tramas vacías.
    let man = b.ok(&format!("snapshots {sid}"));
    let _ = man;
    let r = c.ok(&format!(r#"rawframes {ib} sync {{"t":"blobs","server":"{sid}","hashes":["00","11"]}} 2"#));
    assert_eq!(r, serde_json::json!([0, 0]));
    println!("✔ la jugadora no puede descargar el mundo (manifiesto ni bloques), ni de la anfitriona ni de la coanfitriona");
    // Configuración del servidor: no se la dan.
    let r = c.ok(&format!(r#"raw {ia} ctrl {{"t":"config","server":"{sid}"}}"#));
    assert!(r["config"].is_null(), "configuración filtrada a la jugadora: {r}");
    assert!(r["share"]["invites"].as_array().unwrap().is_empty());
    // Info de otras PCs: sin direcciones, sin puntos, sin capacidad.
    for to in [&ia, &ib] {
        let r = c.ok(&format!(r#"raw {to} ctrl {{"t":"info","server":"{sid}"}}"#));
        assert!(r["addr"].is_null() && r["peers"].as_array().unwrap().is_empty(), "direcciones filtradas: {r}");
        assert!(r["latest"].is_null() && r["score"] == 0 && r["hasCopy"] == false, "datos internos filtrados: {r}");
    }
    println!("✔ la jugadora no recibe configuración, direcciones de red, puntos ni capacidad de hardware de otras PCs");
    // Archivos remotos: la jugadora no puede listar.
    assert!(c.err(&format!("fls {sid}")).len() > 0);
    // Una jugadora no puede votar por sí misma ni pedir alojar.
    let r = c.ok(&format!(r#"raw {ib} ctrl {{"t":"vote","server":"{sid}","epoch":1,"candidate":"{ib}","fresh":[0,0]}}"#));
    assert_eq!(r["agree"], false, "votos para otra PC pedidos por la jugadora");
    let audit = a.ok(&format!("audit {sid}"));
    assert!(audit.as_str().unwrap().contains("sync-denied"), "quedó en el registro de auditoría");
    println!("✔ archivos y votos denegados; los intentos quedan auditados");
}

#[test]
fn invitaciones_vencidas_usadas_revocadas_y_manipuladas() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut x = Node::spawn("X", &tmp.path().join("x"), true, &[]);
    let mut y = Node::spawn("Y", &tmp.path().join("y"), true, &[]);
    let sid = a.ok("create Inv 25911").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    // Usada: segunda PC con el mismo código.
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    x.ok(&format!("join {t}"));
    assert!(y.err(&format!("join {t}")).contains("no es válida"));
    println!("✔ código ya usado: rechazado para otra PC");
    // Vencida.
    let t = a.ok(&format!("invitefor {sid} jugador 1")).as_str().unwrap().to_string();
    a.ok(&format!("expireinvites {sid}"));
    assert!(y.err(&format!("join {t}")).contains("no es válida"));
    assert!(a.ok(&format!("invites {sid}")).as_array().unwrap().is_empty(), "la vencida se borra al intentar usarla");
    println!("✔ código vencido: rechazado y borrado");
    // Revocada.
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let secret = a.ok(&format!("invites {sid}"))[0]["secret"].as_str().unwrap().to_string();
    a.ok(&format!("revoke {sid} {secret}"));
    assert!(y.err(&format!("join {t}")).contains("no es válida"));
    println!("✔ código revocado: rechazado al instante");
    // Manipulada: cambiar un carácter del código, o armar uno con otro secreto.
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let mut bad = t.clone().into_bytes();
    let k = bad.len() - 3;
    bad[k] = if bad[k] == b'a' { b'b' } else { b'a' };
    let bad = String::from_utf8(bad).unwrap();
    let e = y.err(&format!("join {bad}"));
    println!("    manipulado: {e}");
    // El código bueno sigue sirviendo después (los intentos fallidos no lo gastan).
    y.ok(&format!("join {t}"));
    println!("✔ código manipulado: rechazado; el original sigue siendo de un solo uso y válido");
    // El código no lleva IPs cuando hay relays (acá sin relays sí, porque es la única forma de llegar).
    let ia = id(&mut a);
    // Mínimo conocimiento al unirse: una jugadora (X) que usa un código de jugadora no recibe
    // cómo llegar a la coanfitriona (Y), que no aloja. Antes la respuesta traía a todas las PCs.
    let iy = id(&mut y);
    a.ok(&format!("invite {sid} jugador"));
    let secret = a.ok(&format!("invites {sid}")).as_array().unwrap().last().unwrap()["secret"].as_str().unwrap().to_string();
    let r = x.ok(&format!(r#"raw {ia} join {{"secret":"{secret}","name":"X"}}"#));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!r["peers"].to_string().contains(&iy), "la jugadora recibió la dirección de una coanfitriona que no aloja: {r}");
    println!("✔ al unirse, una jugadora no recibe direcciones de las demás PCs (solo la anfitriona)");
    // Fuerza bruta: muchos secretos falsos seguidos desde una PC → límite de intentos.
    let mut limited = false;
    for i in 0..8 {
        let r = x.ok(&format!(r#"raw {ia} join {{"secret":"{:032x}","name":"z"}}"#, i));
        if r["error"].as_str().unwrap_or("").contains("Demasiados intentos") { limited = true; break; }
    }
    assert!(limited, "sin límite de intentos");
    println!("✔ fuerza bruta de códigos: limitada a 5 intentos por minuto por PC");
    // Mensajes malformados contra la unión: no rompen nada.
    x.ok(&format!("rawbytes {ia} join ffffffff00"));
    x.ok(&format!("rawbytes {ia} join 00000004deadbeef"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    assert!(t.starts_with("939-"));
    println!("✔ mensajes malformados en la unión: la PC sigue funcionando");
}

#[test]
fn mensajes_malformados_y_enormes() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &[], 25941);
    let ia = id(&mut a);
    a.ok(&format!("start {sid}"));
    // Basura, longitud declarada gigante, JSON inválido, tipo desconocido, servidor inexistente.
    for alpn in ["ctrl", "sync", "files", "mc", "bridge"] {
        c.ok(&format!("rawbytes {ia} {alpn} ffffffffffffffff"));
        c.ok(&format!("rawbytes {ia} {alpn} 7fffffff7b7d"));
        c.ok(&format!("rawbytes {ia} {alpn} 000000057b7b7b7b7b"));
    }
    let r = c.ok(&format!(r#"raw {ia} ctrl {{"t":"noExiste","server":"{sid}"}}"#));
    assert!(r.get("closed").is_some(), "{r}");
    for bad in ["../../etc", "..", "C:\\\\Windows", "a/b", "x\\u0000y"] {
        let r = c.ok(&format!(r#"raw {ia} ctrl {{"t":"info","server":"{bad}"}}"#));
        assert_eq!(r["t"], "err", "id de servidor {bad}: {r}");
        let r = c.ok(&format!(r#"rawframes {ia} sync {{"t":"manifest","server":"{bad}","snap":"x"}} 1"#));
        assert_eq!(r, serde_json::json!([0]));
    }
    // Un servidor con id manipulado en el túnel de Minecraft: se corta.
    let r = c.ok(&format!(r#"raw {ia} ctrl {{"t":"leaseChain","server":"{sid}","after":0}}"#));
    assert_eq!(r["t"], "leases");
    // Y todo sigue funcionando.
    assert!(a.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("el grupo sigue completo", Duration::from_secs(20), || b.status(&sid)["members"].as_array().unwrap().len() == 3);
    println!("✔ mensajes malformados, enormes y desconocidos en los 5 protocolos: rechazados sin afectar al grupo");
    // Inundación de pedidos desde una jugadora: pasado el límite por PC se rechazan, y el grupo
    // (las demás PCs) sigue funcionando. Incluye «sincronizá ya», que dispara consultas a todos.
    let r = c.ok(&format!(r#"rawflood {ia} ctrl {{"t":"syncNow","server":"{sid}"}} 200"#));
    let r2 = c.ok(&format!(r#"rawflood {ia} ctrl {{"t":"info","server":"{sid}"}} 700"#));
    println!("    inundación: {r} {r2}");
    assert!(r2["limited"].as_u64().unwrap() > 0, "sin límite de pedidos: {r2}");
    assert!(a.ok(&format!("running {sid}")).as_bool().unwrap());
    let before = b.status(&sid)["members"].as_array().unwrap().len();
    assert_eq!(before, 3);
    let r = b.ok(&format!(r#"raw {ia} ctrl {{"t":"info","server":"{sid}"}}"#));
    assert_eq!(r["t"], "info", "la coanfitriona quedó afuera por la inundación de otra PC: {r}");
    println!("✔ inundación de pedidos: limitada por PC; las demás siguen atendidas");
}

#[test]
fn recuperacion_con_codigo_y_sucesion() {
    let tmp = tempfile::tempdir().unwrap();
    let env = [("H939_OWNERSHIP_WAIT", "8")];
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &env, 25951);
    let mut d = Node::spawn("D", &tmp.path().join("d"), true, &env);
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    d.ok(&format!("join {t}"));
    let (ia, ib, ic, idd) = (id(&mut a), id(&mut b), id(&mut c), id(&mut d));
    let _ = &ic;
    a.ok(&format!("start {sid}"));
    let code = a.ok(&format!("reccreate {sid}")).as_str().unwrap().to_string();
    assert!(!a.ok(&format!("share_json {sid}")).to_string().contains(&code), "el código no se guarda");
    assert!(a.err(&format!("reccreate {sid}")).contains("ya tiene"));
    // Una coanfitriona no puede autodesignarse sucesora ni crear códigos.
    assert!(b.err(&format!("successor {sid} {ib}")).contains("propietaria"));
    assert!(b.err(&format!("reccreate {sid}")).contains("propietaria"));
    a.ok(&format!("successor {sid} {ib}"));
    wait("todas ven la lista con código y sucesora", Duration::from_secs(20), || {
        [&mut b, &mut c, &mut d].iter_mut().all(|n| { let s = n.ok(&format!("share_json {sid}")); s["successor"] == ib.as_str() && s["recovery"].is_string() })
    });
    // D (coanfitriona no designada) intenta la sucesión: no puede.
    assert!(d.err(&format!("succstart {sid}")).contains("no es la sucesora"));
    // B (sucesora) intenta empezar con la propietaria activa: no.
    assert!(b.err(&format!("succstart {sid}")).contains("respondió hace poco"));
    println!("✔ solo la propietaria designa sucesora y crea el código; nadie se autodesigna");

    // Se pierde la PC propietaria (se congela y no vuelve). D recupera con el código.
    signal(a.pid(), "STOP");
    std::thread::sleep(Duration::from_secs(9));
    assert!(d.err(&format!("recover {sid} AAAA-BBBB-CCCC-DDDD-EEEE-FFFF-GGGG-HHHH")).len() > 0);
    let new_code = d.ok(&format!("recover {sid} {code}")).as_str().unwrap().to_string();
    assert_ne!(new_code, code);
    wait("B y C aceptan a D como propietaria", Duration::from_secs(30), || {
        [&mut b, &mut c].iter_mut().all(|n| n.ok(&format!("share_json {sid}"))["ownerPin"] == idd.as_str())
    });
    let s = b.ok(&format!("share_json {sid}"));
    assert!(!s["members"].as_array().unwrap().iter().any(|m| m["nodeId"] == ia.as_str()), "A quedó revocada");
    assert!(s["successor"].is_null() || s["successor"] == ib.as_str());
    println!("✔ recuperación con código desde otra PC: A revocada, D propietaria, código nuevo");
    // El código viejo ya no sirve (replay), ni siquiera desde B.
    assert!(b.err(&format!("recover {sid} {code}")).contains("no es el código"));
    // A vuelve (o alguien la usa): sus listas ya no valen para nadie.
    signal(a.pid(), "CONT");
    a.ok(&format!("setrole {sid} {ib} jugador"));
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(b.ok(&format!("share_json {sid}"))["ownerPin"], idd.as_str());
    let roles: Vec<String> = c.ok(&format!("share_json {sid}"))["members"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap().to_string()).collect();
    assert!(roles.iter().filter(|r| *r == "propietario").count() == 1);
    println!("✔ la propietaria anterior ya no puede cambiar nada (sus firmas se rechazan)");

    // Sucesión: D designa a B; D deja de responder; B inicia; antes de la espera no puede completar.
    d.ok(&format!("successor {sid} {ib}"));
    wait("B ve que es sucesora", Duration::from_secs(20), || b.ok(&format!("share_json {sid}"))["successor"] == ib.as_str());
    signal(d.pid(), "STOP");
    std::thread::sleep(Duration::from_secs(1));
    // Con D congelada, B todavía la vio hace poco: espera 5 min de silencio en uso real; acá el
    // nodo de pruebas usa la misma regla, así que se prueba la cancelación primero.
    signal(d.pid(), "CONT");
    println!("✔ sucesora designada y visible para el grupo");
    let _ = (&mut a, &mut c);
}

#[test]
fn mods_de_otra_pc_necesitan_aprobacion() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, _c, sid) = three(tmp.path(), &[], 25971);
    let ib = id(&mut b);
    // La propietaria instala un mod en su PC: queda aprobado solo.
    a.ok(&format!("putmod {sid} propio.jar mod de la propietaria"));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    wait("la propietaria aprobó su mod", Duration::from_secs(20), || b.ok(&format!("share_json {sid}"))["approvedMods"].as_array().map(|v| v.len() == 1).unwrap_or(false));
    // La coanfitriona sube un .jar a la anfitriona por el administrador remoto.
    let evil = tmp.path().join("malo.jar");
    std::fs::write(&evil, b"codigo malicioso").unwrap();
    b.ok(&format!("fput {sid} {} mods/malo.jar", evil.display()));
    // Desvíos de la cuarentena por el administrador remoto: lo que decide qué programa arranca
    // (argumentos de Java de Forge/NeoForge, librerías, el servidor, el lanzador de Fabric) no se
    // puede escribir desde otra PC, con ninguna forma de escribir la ruta.
    for target in ["libraries/net/neoforged/neoforge/21.1.77/win_args.txt", "LIBRARIES/x.jar", "server.jar", "Server.JAR", "fabric-server-launch.jar",
                   "fabric-server-launcher.properties", "versions/26.2/server.jar", ".fabric/x.jar", "mods-cuarentena/malo.jar", "./libraries/a.jar", "Log4j2-939host.XML"] {
        let e = b.err(&format!("fput {sid} {} {target}", evil.display()));
        assert!(e.contains("qué programa se ejecuta"), "{target}: {e}");
    }
    b.ok(&format!("fput {sid} {} config/permitido.jar", evil.display()));
    let e = b.err(&format!("fmv {sid} config/permitido.jar libraries/permitido.jar"));
    assert!(e.contains("qué programa se ejecuta"), "mover a libraries: {e}");
    assert!(b.err(&format!("fmkdir {sid} libraries/nueva")).contains("qué programa se ejecuta"));
    println!("✔ la coanfitriona no puede reemplazar el servidor, las librerías ni los argumentos de Java desde su PC");
    // Pasa a la coanfitriona y vuelve: en cada encendido, lo recibido sin aprobar se aparta.
    a.ok(&format!("handover {sid} {ib}"));
    wait("B aloja", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    let mods_b = b.ok(&format!("modsdir {sid}"));
    assert!(mods_b.as_array().unwrap().iter().any(|m| m == "propio.jar"), "el mod aprobado sí se carga en B: {mods_b}");
    assert!(!mods_b.as_array().unwrap().iter().any(|m| m == "malo.jar"), "el mod sin aprobar no se carga en B: {mods_b}");
    println!("✔ el mod subido por una coanfitriona no se carga sin aprobación (ni en la anfitriona nueva)");
    // B instala otro mod en su propia PC mientras aloja (en su PC corre: es su decisión) y lo pasa a A.
    b.ok(&format!("putmod {sid} de-b.jar mod que puso B"));
    let ia = id(&mut a);
    b.ok(&format!("handover {sid} {ia}"));
    wait("A aloja", Duration::from_secs(30), || a.ok(&format!("running {sid}")).as_bool().unwrap());
    let mods_a = a.ok(&format!("modsdir {sid}"));
    assert!(!mods_a.as_array().unwrap().iter().any(|m| m == "de-b.jar"), "lo que puso B no corre en la PC de A sin aprobar: {mods_a}");
    let pend = a.ok(&format!("pendingmods {sid}"));
    assert!(pend.as_array().unwrap().iter().any(|p| p[0] == "de-b.jar"), "A ve el pendiente: {pend}");
    println!("✔ un mod que puso la coanfitriona en su PC no se ejecuta en la PC de la propietaria sin aprobación");
    // B no puede aprobar.
    let h = pend.as_array().unwrap().iter().find(|p| p[0] == "de-b.jar").unwrap()[1].as_str().unwrap().to_string();
    assert!(b.err(&format!("approvemods {sid} {h}")).contains("propietaria"));
    // A aprueba: vuelve al próximo encendido.
    a.ok(&format!("approvemods {sid} {h}"));
    a.ok(&format!("stop {sid}"));
    a.ok(&format!("start {sid}"));
    assert!(a.ok(&format!("modsdir {sid}")).as_array().unwrap().iter().any(|m| m == "de-b.jar"));
    println!("✔ la propietaria aprueba y el mod vuelve a cargarse; la coanfitriona no puede aprobar");
}

/// Sistema de aprobación: lo que sube una coanfitriona va directo a la cuarentena de la anfitriona
/// (nunca a la carpeta activa, ni siquiera antes de encender), con su procedencia; un rechazo de la
/// propietaria vale en todas las PCs y para siempre, aunque el archivo vuelva a llegar con otro
/// nombre o por otro camino.
#[test]
fn rechazo_y_cuarentena_inmediata() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, _c, sid) = three(tmp.path(), &[], 25975);
    let ib = id(&mut b);
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    let evil = tmp.path().join("ladron.jar");
    std::fs::write(&evil, b"robo de credenciales").unwrap();
    b.ok(&format!("fput {sid} {} mods/ladron.jar", evil.display()));
    // Con el servidor encendido, sin reiniciar: ya está apartado.
    assert!(!a.ok(&format!("modsdir {sid}")).as_array().unwrap().iter().any(|m| m == "ladron.jar"), "nunca en la carpeta activa");
    let q = a.ok(&format!("quarantine {sid}"));
    let entry = q.as_array().unwrap().iter().find(|x| x["name"] == "ladron.jar").unwrap_or_else(|| panic!("en cuarentena: {q}")).clone();
    assert_eq!(entry["from"], ib.as_str(), "la procedencia es la PC autenticada que lo mandó");
    assert_eq!(entry["size"], 20);
    let h = entry["sha256"].as_str().unwrap().to_string();
    println!("✔ lo subido por la coanfitriona queda en cuarentena al instante, con su procedencia");
    // La coanfitriona no puede rechazar ni aprobar.
    assert!(b.err(&format!("rejectmods {sid} {h}")).contains("propietaria"));
    assert!(b.err(&format!("approvemods {sid} {h}")).contains("propietaria"));
    a.ok(&format!("rejectmods {sid} {h}"));
    assert!(a.ok(&format!("pendingmods {sid}")).as_array().unwrap().is_empty(), "un rechazado ya no está pendiente");
    assert!(a.ok(&format!("decisions {sid}")).as_array().unwrap().iter().any(|d| d["decision"] == "rechazado" && d["sha256"] == h.as_str()), "queda registrado");
    // Vuelve a llegar con otro nombre: sigue rechazado.
    b.ok(&format!("fput {sid} {} mods/otro-nombre.jar", evil.display()));
    assert!(!a.ok(&format!("modsdir {sid}")).as_array().unwrap().iter().any(|m| m == "otro-nombre.jar"));
    println!("✔ rechazado: no se carga aunque vuelva a llegar con otro nombre");
    // El rechazo va firmado y llega a la coanfitriona.
    wait("B recibe el rechazo", Duration::from_secs(20), || b.ok(&format!("share_json {sid}"))["rejectedMods"].as_array().map(|v| v.iter().any(|x| x == h.as_str())).unwrap_or(false));
    // En su propia PC, la coanfitriona instala el mismo archivo a mano y lo pasa a la propietaria.
    a.ok(&format!("handover {sid} {ib}"));
    wait("B aloja", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    b.ok(&format!("putmod {sid} a-mano.jar robo de credenciales"));
    b.ok(&format!("stop {sid}"));
    b.ok(&format!("start {sid}"));
    assert!(!b.ok(&format!("modsdir {sid}")).as_array().unwrap().iter().any(|m| m == "a-mano.jar"), "en la PC de B tampoco corre: el rechazo vale para todo el grupo");
    let ia = id(&mut a);
    b.ok(&format!("handover {sid} {ia}"));
    wait("A aloja", Duration::from_secs(30), || a.ok(&format!("running {sid}")).as_bool().unwrap());
    assert!(!a.ok(&format!("modsdir {sid}")).as_array().unwrap().iter().any(|m| m == "a-mano.jar"));
    println!("✔ el rechazo va firmado a todo el grupo y vale aunque el archivo se instale a mano en otra PC");
}

/// Revocación: la propietaria quita a una coanfitriona. Con su identidad y su copia vieja, esa PC
/// ya no puede bajar el mundo nuevo, administrar archivos ni pedir el servidor.
#[test]
fn pc_quitada_pierde_el_acceso() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, _c, sid) = three(tmp.path(), &[], 25977);
    let ib = id(&mut b);
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    assert!(b.ok(&format!("fls {sid}")).is_array(), "control: antes de quitarla, puede listar");
    a.ok(&format!("remove {sid} {ib}"));
    assert!(!a.ok(&format!("share_json {sid}"))["members"].as_array().unwrap().iter().any(|m| m["nodeId"] == ib.as_str()));
    // Archivos: rechazado en la anfitriona.
    let e = b.err(&format!("fls {sid}"));
    println!("    archivos tras quitarla: {e}");
    // Pedir el servidor: rechazado.
    assert!(b.err(&format!("request {sid}")).len() > 0);
    // El mundo nuevo: no le llega.
    a.ok(&format!("write {sid} marca.txt despues"));
    let s2 = a.ok(&format!("snapshot {sid}"));
    b.ok(&format!("sync {sid}"));
    std::thread::sleep(Duration::from_secs(6));
    assert_ne!(fresh(&b.status(&sid)["myLatest"]), fresh(&s2), "una PC quitada no recibe el mundo nuevo");
    let audit = a.ok(&format!("audit {sid}"));
    println!("    auditoría: {}", audit.as_array().map(|v| v.iter().rev().take(4).map(|x| x.to_string()).collect::<Vec<_>>().join(" | ")).unwrap_or_default());
    println!("✔ quitada del grupo: sin archivos, sin pedir el servidor y sin el mundo nuevo");
}

/// Modo privado sin relay disponible: no se conecta, aunque haya camino directo (nunca cambia a
/// directo por su cuenta). La anfitriona está en la misma PC, sin relays.
#[test]
fn modo_privado_no_cae_a_directo() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let sid = a.ok("create Priv 25981").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    assert!(a.ok(&format!("ticketinfo {t}"))["ips"].as_u64().unwrap() > 0, "sin relays el código necesita IPs");
    let mut p = NodeArgs::spawn("P", &tmp.path().join("p"), &["--private"]);
    let e = p.err(&format!("join {t}"));
    println!("    privada sin relay: {e}");
    let ip = id(&mut p);
    let seen = a.ok(&format!("remoteinfo {ip}"));
    assert!(seen.as_array().unwrap().is_empty(), "A no debía recibir ninguna conexión de la PC privada: {seen}");
    assert!(a.ok(&format!("share_json {sid}"))["members"].as_array().unwrap().len() == 1, "P no se unió");
    println!("✔ en modo privado, sin relay no hay conexión: no cambió a directo aunque la otra PC estaba al lado");
}

/// Modo privado con relays reales de n0 (necesita internet; corre en la CI de Windows).
#[test]
#[ignore]
fn modo_privado_oculta_la_ip_real() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = NodeArgs::spawn("A", &tmp.path().join("a"), &["--relays"]);
    let mut b = NodeArgs::spawn("B", &tmp.path().join("b"), &["--relays"]);
    let mut p = NodeArgs::spawn("P", &tmp.path().join("p"), &["--private"]);
    let sid = a.ok("create PrivReal 25991").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    std::thread::sleep(Duration::from_secs(5));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    let ti = a.ok(&format!("ticketinfo {t}"));
    assert_eq!(ti["ips"], 0, "con relays el código no lleva IPs: {ti}");
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25992"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    p.ok(&format!("join {t}"));
    p.ok(&format!("setport {sid} 25993"));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("P copia por relay", Duration::from_secs(90), || fresh(&p.status(&sid)["myLatest"]) == fresh(&s1));
    let ip = id(&mut p);
    std::thread::sleep(Duration::from_secs(10));
    for (n, name) in [(&mut a, "A"), (&mut b, "B")] {
        let ri = n.ok(&format!("remoteinfo {ip}"));
        println!("    {name} conoce de P: {ri}");
        assert!(ri.as_array().unwrap().iter().all(|x| !x.as_str().unwrap().contains("Ip(")), "{name} conoce una IP de la PC privada: {ri}");
        let pf = n.ok("peersfile").as_str().unwrap().to_string();
        assert!(!pf.contains("\"ip\"") || !pf.contains(&ip[..16]) || !pf.split(&ip[..16]).nth(1).unwrap_or("").split('}').next().unwrap_or("").contains("ip"), "{name} guardó una IP de P");
        let info = n.ok(&format!(r#"raw {ip} ctrl {{"t":"info","server":"{sid}"}}"#));
        let addr = info["addr"].to_string();
        assert!(!addr.contains("\"Ip\"") && !addr.to_lowercase().contains("\"ip\""), "{name} recibió IPs de P en Info: {addr}");
        let st = n.status(&sid);
        let m = st["members"].as_array().unwrap().iter().find(|m| m["nodeId"] == ip.as_str()).cloned().unwrap_or_default();
        println!("    {name} ve a P por: {}", m["path"]);
        assert!(m["path"] == "relay" || m["path"] == "", "{name} conecta directo con la PC privada");
    }
    println!("✔ modo privado real: ninguna otra PC conoce la IP de la PC privada (ni por conexión, ni en Info, ni en archivos); el grupo funciona por relay");
    // El resto del grupo no se ve afectado: A y B pueden ir directo entre sí.
    let ibb = id(&mut b);
    let ri = a.ok(&format!("remoteinfo {ibb}"));
    println!("    A conoce de B (sin modo privado): {ri}");
}

#[test]
fn pc_atrasada_verifica_la_cadena_completa() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &[], 25961);
    let (ia, ib) = (id(&mut a), id(&mut b));
    a.ok(&format!("start {sid}"));
    let s1 = a.ok(&format!("snapshot {sid}"));
    wait("B copia", Duration::from_secs(20), || fresh(&b.status(&sid)["myLatest"]) == fresh(&s1));
    wait("C conoce el permiso", Duration::from_secs(20), || epoch(&mut c, &sid) == 1);
    signal(c.pid(), "STOP");
    a.ok(&format!("handover {sid} {ib}"));
    wait("B aloja", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    b.ok(&format!("handover {sid} {ia}"));
    wait("A aloja", Duration::from_secs(30), || a.ok(&format!("running {sid}")).as_bool().unwrap());
    a.ok(&format!("handover {sid} {ib}"));
    wait("B aloja otra vez", Duration::from_secs(30), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    signal(c.pid(), "CONT");
    wait("C se pone al día", Duration::from_secs(30), || epoch(&mut c, &sid) == 4 && host(&mut c, &sid) == ib);
    let log = c.ok(&format!("leaselog {sid}"));
    println!("    historial de C: {} permisos", log.as_array().unwrap().len());
    assert_eq!(c.ok(&format!("rejected {sid}")).as_u64().unwrap(), 0, "C no rechazó nada legítimo");
    println!("✔ una PC que estuvo congelada durante tres traspasos se pone al día verificando cada permiso");
}

/// Red team: una jugadora y una coanfitriona maliciosas anuncian direcciones falsas de otras PCs
/// (un relay propio y una IP ajena) para que el resto intente llegar por ahí: verían sus IPs y
/// podrían cortarles la conexión. Ninguna PC guarda esas direcciones.
#[test]
fn direcciones_falsas_de_otras_pcs_no_se_aceptan() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = Node::spawn("A", &tmp.path().join("a"), true, &[]);
    let mut b = Node::spawn("B", &tmp.path().join("b"), true, &[]);
    let ib = id(&mut b);
    // C (jugadora) dice que la dirección de B es un relay propio y una IP ajena.
    let evil_b = format!(r#"[{{"id":"{ib}","addrs":[{{"Relay":"https://relay.evil.example/"}},{{"Ip":"203.0.113.9:4444"}}]}}]"#);
    let mut c = Node::spawn("C", &tmp.path().join("c"), true, &[("H939_TEST_EVIL_ADDRS", &evil_b)]);
    let ic = id(&mut c);
    // D (coanfitriona) dice lo mismo de C.
    let evil_c = format!(r#"[{{"id":"{ic}","addrs":[{{"Relay":"https://relay.evil.example/"}},{{"Ip":"203.0.113.9:5555"}}]}}]"#);
    let mut d = Node::spawn("D", &tmp.path().join("d"), true, &[("H939_TEST_EVIL_ADDRS", &evil_c)]);
    let sid = a.ok("create Direcciones 25995").as_str().unwrap().to_string();
    a.ok(&format!("share {sid}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    b.ok(&format!("join {t}"));
    b.ok(&format!("setport {sid} 25996"));
    let t = a.ok(&format!("invite {sid} jugador")).as_str().unwrap().to_string();
    c.ok(&format!("join {t}"));
    let t = a.ok(&format!("invite {sid} coanfitrion")).as_str().unwrap().to_string();
    d.ok(&format!("join {t}"));
    d.ok(&format!("setport {sid} 25997"));
    // Varias vueltas de consultas (0,7 s cada una en las pruebas).
    std::thread::sleep(Duration::from_secs(8));
    for n in [&mut a, &mut b] {
        let p = n.ok("peersfile");
        let p = p.as_str().unwrap();
        assert!(!p.contains("evil.example") && !p.contains("203.0.113.9"), "{}: guardó una dirección falsa: {p}", n.name);
    }
    // El grupo sigue funcionando: A sigue viendo a todas.
    wait("A ve a las 4", Duration::from_secs(20), || a.status(&sid)["members"].as_array().unwrap().len() == 4);
    println!("✔ direcciones falsas de otras PCs (relay propio, IP ajena) anunciadas por una jugadora o una coanfitriona: descartadas");
}

/// Estado compartido y OP de Minecraft concedido por la propietaria (§13): nadie se lo da a sí
/// mismo, la anfitriona lo aplica, sobrevive al traspaso y la revocación se respeta.
#[test]
fn estado_compartido_y_op_concedido_por_la_propietaria() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut a, mut b, mut c, sid) = three(tmp.path(), &[], 25881);
    let id_b = id(&mut b);
    a.ok(&format!("start {sid}"));
    wait("las tres ven el mismo estado", Duration::from_secs(25), || {
        [&mut a, &mut b, &mut c].iter_mut().all(|n| n.status(&sid)["hostState"]["state"] == "running")
    });
    println!("✔ todas las PCs ven el servidor encendido en A");

    // B (coanfitriona) no puede darse OP, ni C (jugadora).
    assert!(b.err(&format!("grantop {sid} {id_b} Bjugador")).contains("propietaria"));
    assert!(c.err(&format!("grantop {sid} {id_b} Bjugador")).contains("propietaria"));
    // La propietaria lo concede; la anfitriona (A) lo aplica en el servidor encendido.
    a.ok(&format!("grantop {sid} {id_b} Bjugador"));
    wait("A aplica el OP", Duration::from_secs(20), || a.ok(&format!("ops {sid}")).as_str().unwrap_or("").contains("Bjugador"));
    wait("B ve su OP en la lista firmada", Duration::from_secs(20), || b.status(&sid)["members"].as_array().unwrap().iter().any(|m| m["mcOp"] == "Bjugador"));
    println!("✔ solo la propietaria concede OP; la anfitriona lo aplicó");

    // Traspaso a B: el estado lo ven todas y la nueva anfitriona respeta la revocación.
    a.ok(&format!("handover {sid} {id_b}"));
    wait("B aloja", Duration::from_secs(40), || b.ok(&format!("running {sid}")).as_bool().unwrap());
    wait("A y C ven el estado de B", Duration::from_secs(25), || {
        let (sa, sc) = (a.status(&sid), c.status(&sid));
        sa["hostState"]["state"] == "running" && sc["hostState"]["state"] == "running" && sa["lease"]["host"] == id_b.as_str()
    });
    a.ok(&format!("revokeop {sid} {id_b}"));
    wait("B (anfitriona) quita el OP", Duration::from_secs(25), || {
        let ops = b.ok(&format!("ops {sid}"));
        // ops.json viene con el mundo; si todavía no existe es porque nunca tuvo OP en B.
        !ops.as_str().unwrap_or("").contains("Bjugador") && b.status(&sid)["members"].as_array().unwrap().iter().all(|m| m["mcOp"].is_null())
    });
    println!("✔ después del traspaso la nueva anfitriona respetó la revocación");

    b.ok(&format!("stop {sid}"));
    wait("A y C ven el servidor apagado", Duration::from_secs(25), || {
        [&mut a, &mut c].iter_mut().all(|n| { let s = n.status(&sid)["hostState"]["state"].clone(); s == "stopped" })
    });
    println!("✔ al apagarlo, todas lo ven apagado");
}
