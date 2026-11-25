//! Direcciones `.939host` de verdad en Windows: escribe el bloque en el archivo hosts, resuelve
//! el nombre con el sistema (como Java/Minecraft), escucha en 127.93.x.y:25565, conecta por
//! nombre y deja el archivo hosts exactamente como estaba. Solo con H939_TEST_HOSTS=1 (CI).

use host939_core::alias::{self, Entry};
use std::net::ToSocketAddrs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn direccion_resuelve_y_conecta() {
    if std::env::var("H939_TEST_HOSTS").ok().as_deref() != Some("1") { eprintln!("omitida: H939_TEST_HOSTS=1 solo en CI"); return; }
    let before = std::fs::read(alias::hosts_path()).unwrap();
    let ip = alias::ip_for("prueba-ci-939");
    let e = vec![Entry { label: "prueba-ci".into(), ip }];
    alias::write_hosts(&e).unwrap();
    let res = tokio::spawn(async move {
        let got: Vec<_> = ("prueba-ci.939host", 25565).to_socket_addrs().expect("el sistema no resolvió el nombre").collect();
        println!("  prueba-ci.939host → {got:?}");
        assert!(got.iter().any(|a| a.ip() == std::net::IpAddr::V4(ip)));
        let srv = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = srv.local_addr().unwrap().port();
        tokio::spawn(async move { let (mut s, _) = srv.accept().await.unwrap(); let mut b = [0u8; 9]; s.read_exact(&mut b).await.unwrap(); s.write_all(&b).await.unwrap(); });
        let h = alias::forward(ip, move || Some(p)).await.expect("no se pudo escuchar en 127.93.x.y:25565");
        let mut c = tokio::net::TcpStream::connect("prueba-ci.939host:25565").await.unwrap();
        c.write_all(b"939-hosts").await.unwrap();
        let mut b = [0u8; 9]; c.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"939-hosts");
        h.abort();
        println!("✔ conexión por nombre llevada al servidor local");
    });
    let r = res.await.map_err(|e| e.into_panic());
    alias::write_hosts(&[]).unwrap();
    let after = std::fs::read(alias::hosts_path()).unwrap();
    assert_eq!(String::from_utf8_lossy(&after).trim_end(), String::from_utf8_lossy(&before).trim_end(), "el archivo hosts no quedó como estaba");
    println!("✔ archivo hosts restaurado");
    if let Err(p) = r { std::panic::resume_unwind(p); }
}


/// El script que Windows ejecuta como administrador (PowerShell de System32) deja el archivo hosts
/// igual que la lógica de Rust, y rechaza entradas fuera de 939 Host. Solo con H939_TEST_HOSTS=1
/// (CI de Windows, que corre como administrador).
#[test]
fn script_elevado_escribe_igual_que_rust() {
    if std::env::var("H939_TEST_HOSTS").ok().as_deref() != Some("1") { eprintln!("omitida: H939_TEST_HOSTS=1 solo en CI"); return; }
    let run = |script: &str| std::process::Command::new(alias::powershell_path())
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &alias::powershell_encoded(script)]).status().unwrap();
    let before = String::from_utf8_lossy(&std::fs::read(alias::hosts_path()).unwrap()).to_string();
    let e = vec![Entry { label: "zeta-ps".into(), ip: alias::ip_for("ps-a") }, Entry { label: "alfa-ps".into(), ip: alias::ip_for("ps-b") }];
    assert!(run(&alias::hosts_script(&e).unwrap()).success());
    let now = String::from_utf8_lossy(&std::fs::read(alias::hosts_path()).unwrap()).to_string();
    assert_eq!(now, alias::apply(&before, &e), "el script de PowerShell y la lógica de Rust tienen que dar lo mismo");
    // Un script alterado con una entrada ajena (redirigir otro dominio) no escribe nada.
    let bad = alias::hosts_script(&e).unwrap().replace("'127.93.", "'203.0.113.").replace("alfa-ps'", "ejemplo.com'");
    assert!(!run(&bad).success(), "tenía que rechazar la entrada ajena");
    assert_eq!(String::from_utf8_lossy(&std::fs::read(alias::hosts_path()).unwrap()), now, "no cambió nada");
    assert!(run(&alias::hosts_script(&[]).unwrap()).success());
    let after = String::from_utf8_lossy(&std::fs::read(alias::hosts_path()).unwrap()).to_string();
    assert_eq!(after.trim_end(), before.trim_end(), "el archivo hosts no quedó como estaba");
    println!("✔ el script elevado escribe igual que Rust, rechaza entradas ajenas y deja el archivo como estaba");
}
