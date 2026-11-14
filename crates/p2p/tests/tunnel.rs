//! Pruebas reales del túnel en esta máquina (dos nodos, conexión directa por loopback).

use host939_p2p::{client, host, Allowlist, Relays};
use iroh::SecretKey;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn key(n: u8) -> [u8; 32] { [n; 32] }
fn id(n: u8) -> iroh::EndpointId { SecretKey::from_bytes(&key(n)).public() }

async fn echo_server() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                loop { let n = s.read(&mut buf).await.unwrap_or(0); if n == 0 { break; } s.write_all(&buf[..n]).await.unwrap(); }
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn authorized_member_tunnels_and_stranger_is_rejected() {
    let target = echo_server().await;
    let allow = Allowlist::default();
    allow.allow(id(2));
    let h = host(key(1), target, Relays::Disabled, allow).await.unwrap();

    // Miembro autorizado: 1 MiB de ida y vuelta, en dos conexiones simultáneas.
    let c = client(key(2), h.addr(), 0, Relays::Disabled).await.unwrap();
    let mut tasks = vec![];
    for k in 0..2u8 {
        let port = c.local_port;
        tasks.push(tokio::spawn(async move {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let data: Vec<u8> = (0..1_048_576u32).map(|i| (i as u8) ^ k).collect();
            let (mut r, mut w) = s.split();
            let send = async { w.write_all(&data).await.unwrap(); };
            let mut got = vec![0u8; data.len()];
            let recv = async { r.read_exact(&mut got).await.unwrap(); };
            tokio::join!(send, recv);
            assert_eq!(got, data);
        }));
    }
    for t in tasks { tokio::time::timeout(Duration::from_secs(30), t).await.unwrap().unwrap(); }
    println!("✔ miembro autorizado: 2 conexiones de 1 MiB ida y vuelta íntegras");

    // Desconocido: el anfitrión cierra la conexión y no llega nada.
    let stranger = client(key(3), h.addr(), 0, Relays::Disabled).await;
    if let Ok(st) = stranger {
        let mut s = TcpStream::connect(("127.0.0.1", st.local_port)).await.unwrap();
        let _ = s.write_all(b"hola").await;
        let mut buf = [0u8; 4];
        let res = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf)).await;
        assert!(!matches!(res, Ok(Ok(4))), "un nodo no autorizado no debe recibir datos");
    }
    println!("✔ nodo no autorizado rechazado");
    c.shutdown().await;
    h.shutdown().await;
}

fn varint(mut v: i32, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7F) as u8;
        v = ((v as u32) >> 7) as i32;
        if v != 0 { b |= 0x80; }
        out.push(b);
        if v == 0 { break; }
    }
}
async fn read_varint(s: &mut TcpStream) -> i32 {
    let (mut val, mut shift) = (0i32, 0);
    loop { let b = s.read_u8().await.unwrap(); val |= ((b & 0x7F) as i32) << shift; if b & 0x80 == 0 { return val; } shift += 7; }
}

/// Ping de estado de Minecraft (lo que hace la lista de servidores del juego) a través del túnel.
async fn mc_status(port: u16) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut hs = vec![0x00];
    varint(-1, &mut hs);
    varint(9, &mut hs); hs.extend_from_slice(b"localhost");
    hs.extend_from_slice(&port.to_be_bytes());
    varint(1, &mut hs);
    let mut pkt = vec![]; varint(hs.len() as i32, &mut pkt); pkt.extend(hs);
    pkt.extend_from_slice(&[0x01, 0x00]);
    s.write_all(&pkt).await.unwrap();
    let _len = read_varint(&mut s).await;
    let _id = read_varint(&mut s).await;
    let n = read_varint(&mut s).await as usize;
    let mut json = vec![0u8; n];
    s.read_exact(&mut json).await.unwrap();
    String::from_utf8(json).unwrap()
}

/// Requiere HOST939_TEST_SERVER_DIR: carpeta con un servidor 26.2 YA en marcha en el puerto 25613.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn real_minecraft_status_through_tunnel() {
    let allow = Allowlist::default();
    allow.allow(id(2));
    let h = host(key(1), 25613, Relays::Disabled, allow).await.unwrap();
    let c = client(key(2), h.addr(), 41939, Relays::Disabled).await.unwrap();
    let direct = mc_status(25613).await;
    let tunneled = tokio::time::timeout(Duration::from_secs(20), mc_status(c.local_port)).await.expect("timeout");
    println!("directo:   {direct}\npor túnel: {tunneled}");
    let v: serde_json::Value = serde_json::from_str(&tunneled).unwrap();
    assert_eq!(v["version"]["name"], "26.2");
    assert!(tunneled.contains("Prueba P2P"));
    println!("✔ el servidor real respondió a través del túnel P2P en 127.0.0.1:{}", c.local_port);
}

#[tokio::test(flavor = "multi_thread")]
async fn invitation_code_authorizes_and_revocation_blocks() {
    use host939_p2p::{host_with_invites, join, make_ticket, parse_ticket, Invites, ServerCard};
    use std::sync::{Arc, Mutex};
    let target = echo_server().await;
    let joined: Arc<Mutex<Vec<String>>> = Default::default();
    let j2 = joined.clone();
    let invites = Invites::default();
    let card = ServerCard { server_id: "zombies-1a2b".into(), name: "939-ZOMBIES".into(), mc_version: "26.2".into(), loader: "fabric".into() };
    invites.add("s3cr3t".into(), card.clone());
    let h = host_with_invites(key(10), target, Relays::Disabled, Allowlist::default(), invites.clone(), Arc::new(move |_id, name, _c| j2.lock().unwrap().push(name))).await.unwrap();

    let ticket = make_ticket(&h.addr(), "s3cr3t");
    assert!(ticket.starts_with("939-"));
    assert_eq!(parse_ticket(&ticket).unwrap().1, "s3cr3t");
    assert!(parse_ticket("939-dañado").is_err());

    // Antes de unirse: el túnel no lleva datos.
    // Con el código: queda autorizado y recibe los datos del servidor.
    let (addr, got) = join(key(11), &ticket, "Alex", Relays::Disabled).await.unwrap();
    assert_eq!(got, card);
    assert_eq!(joined.lock().unwrap().as_slice(), ["Alex"]);
    let c = client(key(11), addr, 0, Relays::Disabled).await.unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", c.local_port)).await.unwrap();
    s.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tokio::time::timeout(Duration::from_secs(10), s.read_exact(&mut buf)).await.unwrap().unwrap();
    assert_eq!(&buf, b"ping");
    println!("✔ código de invitación: autoriza a Alex y el túnel funciona");

    // Código revocado: otro nodo ya no puede unirse.
    invites.revoke("s3cr3t");
    let err = join(key(12), &ticket, "Intruso", Relays::Disabled).await.unwrap_err();
    assert!(err.to_string().contains("inválido"), "{err}");
    println!("✔ código revocado: rechazado ({err})");
    c.shutdown().await;
    h.shutdown().await;
}
