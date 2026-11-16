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

