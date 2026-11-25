//! Pruebas del sistema de actualizaciones contra los ataques que describe TUF y los errores de uso.
//! Todo con claves y archivos descartables en carpetas temporales.

use chrono::{Duration, Utc};
use host939_update::client::{Client, Decision, State};
use host939_update::download::{self, DownloadError};
use host939_update::keys::{SecretKey, Signer};
use host939_update::metadata::{Artifact, Envelope, Error, Release};
use host939_update::publish::{self, Evidence, Policy};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Owner { device: SecretKey, recovery: SecretKey }

fn owner() -> Owner { Owner { device: SecretKey::generate_p256(), recovery: SecretKey::generate_ed25519() } }

fn root_v(o: &Owner, version: u64, device: &SecretKey, signers: &[&dyn Signer], days: i64) -> Vec<u8> {
    let r = publish::root(version, Utc::now() + Duration::days(days), &[device.public(), o.recovery.public()], 1, &[device.public()], 1);
    publish::sign_root(&r, signers).unwrap()
}

fn release(channel: &str, seq: u64, version: &str, data: u32, payload: &[u8]) -> Release {
    Release {
        kind: "release".into(), channel: channel.into(), seq, published: Utc::now(), expires: Utc::now() + Duration::days(30),
        version: version.into(), data_version: data, rollback: false,
        artifacts: vec![Artifact { name: "939Host-x-instalador.exe".into(), size: payload.len() as u64, sha256: hex::encode(Sha256::digest(payload)) }],
        notes: String::new(),
    }
}

fn signed(r: &Release, s: &dyn Signer) -> Vec<u8> { publish::sign_release(r, &[s]).unwrap() }

#[test]
fn actualizacion_correcta_y_canales() {
    let o = owner();
    let root1 = root_v(&o, 1, &o.device, &[&o.device], 365);
    let c = Client::new(&root1, State::default()).unwrap();
    let st = signed(&release("estable", 1, "1.1.0", 1, b"x"), &o.device);
    let be = signed(&release("beta", 1, "1.2.0-beta.1", 1, b"y"), &o.device);
    match c.decide("estable", Some(&st), Some(&be), "1.0.0", 1, Utc::now()).unwrap() {
        Decision::Offer(r) => assert_eq!(r.version, "1.1.0", "Estable nunca recibe Beta aunque esté disponible y firmada"),
        d => panic!("{d:?}"),
    }
    match c.decide("beta", Some(&st), Some(&be), "1.0.0", 1, Utc::now()).unwrap() {
        Decision::Offer(r) => assert_eq!(r.version, "1.2.0-beta.1"),
        d => panic!("{d:?}"),
    }
    assert_eq!(c.decide("estable", Some(&st), None, "1.1.0", 1, Utc::now()).unwrap(), Decision::UpToDate);
    // Un metadato de Beta servido como si fuera Estable se rechaza.
    assert_eq!(c.decide("estable", Some(&be), None, "1.0.0", 1, Utc::now()), Err(Error::Channel));
    // Estable con una versión «-beta» o Beta sin ella: inválidos.
    assert_eq!(c.decide("estable", Some(&signed(&release("estable", 1, "1.2.0-beta.1", 1, b"x"), &o.device)), None, "1.0.0", 1, Utc::now()), Err(Error::Channel));
}

#[test]
fn firma_invalida_clave_ajena_tipo_cambiado_y_firma_repetida() {
    let o = owner();
    let root1 = root_v(&o, 1, &o.device, &[&o.device], 365);
    let c = Client::new(&root1, State::default()).unwrap();
    let r = release("estable", 1, "1.1.0", 1, b"x");
    // Clave que no es del proyecto.
    let intruder = SecretKey::generate_p256();
    assert!(matches!(c.decide("estable", Some(&signed(&r, &intruder)), None, "1.0.0", 1, Utc::now()), Err(Error::Signatures { .. })));
    // Contenido cambiado después de firmar.
    let mut env: Envelope = serde_json::from_slice(&signed(&r, &o.device)).unwrap();
    let mut tampered = r.clone();
    tampered.artifacts[0].sha256 = hex::encode(Sha256::digest(b"malicioso"));
    env.payload = base64_of(&serde_json::to_vec_pretty(&tampered).unwrap());
    assert!(matches!(c.decide("estable", Some(&serde_json::to_vec(&env).unwrap()), None, "1.0.0", 1, Utc::now()), Err(Error::Signatures { .. })));
    // Una raíz firmada no sirve como publicación (separación de dominio por tipo).
    let mut as_release: Envelope = serde_json::from_slice(&root1).unwrap();
    as_release.kind = "release".into();
    assert!(c.decide("estable", Some(&serde_json::to_vec(&as_release).unwrap()), None, "1.0.0", 1, Utc::now()).is_err());
    // Umbral 2: la misma firma repetida no cuenta dos veces.
    let k2 = SecretKey::generate_p256();
    let rr = publish::root(1, Utc::now() + Duration::days(9), &[o.device.public()], 1, &[o.device.public(), k2.public()], 2);
    let c2 = Client::new(&publish::sign_root(&rr, &[&o.device]).unwrap(), State::default()).unwrap();
    let mut e: Envelope = serde_json::from_slice(&signed(&r, &o.device)).unwrap();
    e.signatures.push(e.signatures[0].clone());
    assert_eq!(c2.decide("estable", Some(&serde_json::to_vec(&e).unwrap()), None, "1.0.0", 1, Utc::now()), Err(Error::Signatures { needed: 2, valid: 1 }));
    let both = publish::sign_release(&r, &[&o.device, &k2]).unwrap();
    assert!(matches!(c2.decide("estable", Some(&both), None, "1.0.0", 1, Utc::now()), Ok(Decision::Offer(_))));
}

fn base64_of(b: &[u8]) -> String { use base64::Engine; base64::engine::general_purpose::STANDARD.encode(b) }

#[test]
fn vencidos_retroceso_y_repeticion() {
    let o = owner();
    let root1 = root_v(&o, 1, &o.device, &[&o.device], 365);
    let mut c = Client::new(&root1, State::default()).unwrap();
    // Metadatos vencidos (congelamiento): no se aceptan aunque la firma sea buena.
    let mut old = release("estable", 1, "1.1.0", 1, b"x");
    old.expires = Utc::now() - Duration::hours(1);
    assert_eq!(c.decide("estable", Some(&signed(&old, &o.device)), None, "1.0.0", 1, Utc::now()), Err(Error::Expired));
    // Se acepta la secuencia 5 y se recuerda.
    let r5 = signed(&release("estable", 5, "1.3.0", 1, b"x"), &o.device);
    c.remember("estable", &r5, Utc::now()).unwrap();
    // Repetición de una publicación vieja (secuencia menor) válida y sin vencer: rechazada.
    let r4 = signed(&release("estable", 4, "1.2.0", 1, b"x"), &o.device);
    assert!(matches!(c.decide("estable", Some(&r4), None, "1.0.0", 1, Utc::now()), Err(Error::Rollback(_))));
    // Misma secuencia con otro contenido (dos versiones distintas firmadas con el mismo número): rechazada.
    let r5b = signed(&release("estable", 5, "1.3.1", 1, b"x"), &o.device);
    assert!(matches!(c.decide("estable", Some(&r5b), None, "1.0.0", 1, Utc::now()), Err(Error::Rollback(_))));
    // La misma publicación ya vista: sin error.
    assert!(c.decide("estable", Some(&r5), None, "1.3.0", 1, Utc::now()).is_ok());
    // Una versión menor sin marcar como vuelta atrás nunca se ofrece (downgrade).
    let lower = signed(&release("estable", 6, "1.2.0", 1, b"x"), &o.device);
    assert_eq!(c.decide("estable", Some(&lower), None, "1.3.0", 1, Utc::now()).unwrap(), Decision::UpToDate);
    // El estado guardado con una raíz inventada no introduce claves.
    let mut st = c.state.clone();
    st.root = serde_json::to_string(&vec![String::from_utf8(root_v(&o, 2, &SecretKey::generate_p256(), &[&SecretKey::generate_p256()], 9)).unwrap()]).unwrap();
    assert!(Client::new(&root1, st).is_err());
}

#[test]
fn vuelta_atras_firmada_y_datos_migrados() {
    let o = owner();
    let root1 = root_v(&o, 1, &o.device, &[&o.device], 365);
    let c = Client::new(&root1, State::default()).unwrap();
    let mut rb = release("estable", 7, "1.2.0", 1, b"x");
    rb.rollback = true;
    let rb_bytes = signed(&rb, &o.device);
    // Con datos todavía en formato 1: se ofrece la vuelta atrás.
    assert!(matches!(c.decide("estable", Some(&rb_bytes), None, "1.3.0", 1, Utc::now()), Ok(Decision::Offer(_))));
    // Si la versión instalada ya migró los datos a formato 2, volver atrás el programa no alcanza.
    assert!(matches!(c.decide("estable", Some(&rb_bytes), None, "1.3.0", 2, Utc::now()), Ok(Decision::Blocked(_))));
    // Una versión nueva con un formato de datos menor al actual también se bloquea.
    let weird = signed(&release("estable", 8, "1.4.0", 1, b"x"), &o.device);
    assert!(matches!(c.decide("estable", Some(&weird), None, "1.3.0", 2, Utc::now()), Ok(Decision::Blocked(_))));
}

#[test]
fn rotacion_revocacion_y_recuperacion() {
    let o = owner();
    let root1 = root_v(&o, 1, &o.device, &[&o.device], 365);
    let new_device = SecretKey::generate_p256();
    // Cambio de PC normal: firman la clave vieja y la nueva.
    let root2 = root_v(&o, 2, &new_device, &[&o.device, &new_device], 365);
    let mut c = Client::new(&root1, State::default()).unwrap();
    let r9 = signed(&release("estable", 9, "1.1.0", 1, b"x"), &o.device);
    c.remember("estable", &r9, Utc::now()).unwrap();
    assert!(c.update_root(|n| (n == 2).then(|| root2.clone()), Utc::now()).unwrap());
    assert_eq!(c.root.version, 2);
    // La clave vieja quedó revocada: lo que firme ya no se acepta.
    let by_old = signed(&release("estable", 10, "1.2.0", 1, b"x"), &o.device);
    assert!(matches!(c.decide("estable", Some(&by_old), None, "1.0.0", 1, Utc::now()), Err(Error::Signatures { .. })));
    // Al cambiar las claves de publicación se olvidan las secuencias (una clave comprometida pudo
    // publicar una secuencia enorme): la clave nueva puede volver a empezar.
    let by_new = signed(&release("estable", 1, "1.2.0", 1, b"x"), &new_device);
    assert!(matches!(c.decide("estable", Some(&by_new), None, "1.0.0", 1, Utc::now()), Ok(Decision::Offer(_))));
    // PC perdida: la clave de recuperación sola firma la raíz siguiente con un dispositivo nuevo.
    let third = SecretKey::generate_p256();
    let root3 = root_v(&o, 3, &third, &[&o.recovery, &third], 365);
    assert!(c.update_root(|n| (n == 3).then(|| root3.clone()), Utc::now()).unwrap());
    // Un atacante con una clave cualquiera no puede encadenar una raíz.
    let evil = SecretKey::generate_p256();
    let root4 = root_v(&o, 4, &evil, &[&evil], 365);
    assert!(matches!(c.update_root(|n| (n == 4).then(|| root4.clone()), Utc::now()), Err(Error::Signatures { .. })));
    // Ni saltear versiones.
    let root5 = root_v(&o, 5, &third, &[&third], 365);
    assert!(matches!(c.update_root(|n| (n == 4).then(|| root5.clone()), Utc::now()), Err(Error::Version(_))));
    // Raíz final vencida: no se confía (congelamiento con una raíz vieja).
    let mut c2 = Client::new(&root1, State::default()).unwrap();
    let expired = root_v(&o, 2, &new_device, &[&o.device, &new_device], -1);
    assert_eq!(c2.update_root(|n| (n == 2).then(|| expired.clone()), Utc::now()), Err(Error::Expired));
    // El estado guardado reconstruye la cadena completa desde la raíz de la app.
    let c3 = Client::new(&root1, c.state.clone()).unwrap();
    assert_eq!(c3.root.version, 3);
}

#[test]
fn controles_antes_de_autorizar() {
    let policy = Policy { require_authenticode: true, ..Policy::default() };
    let mut ev = Evidence { commit: "a".repeat(40), branch: "main".into(), jobs: Default::default(), authenticode: "NotSigned".into(), critical: vec![] };
    let draft = release("estable", 2, "1.1.0", 1, b"inst");
    let art = draft.artifacts[0].clone();
    let files = vec![(art.clone(), format!("{}  {}", art.sha256, art.name))];
    let prev = release("estable", 1, "1.0.0", 1, b"old");
    let b = publish::gate(&policy, &ev, Some(&prev), &draft, &files);
    assert!(b.iter().any(|x| x.contains("Falta el resultado")), "{b:?}");
    assert!(b.iter().any(|x| x.contains("Authenticode")), "{b:?}");
    for j in &policy.required_jobs { ev.jobs.insert(j.clone(), "success".into()); }
    ev.authenticode = "Valid".into();
    assert!(publish::gate(&policy, &ev, Some(&prev), &draft, &files).is_empty());
    // CI con un trabajo fallado, otra rama, error crítico, versión no mayor, archivo distinto o ausente del SHA256SUMS.
    let mut ev2 = ev.clone();
    ev2.jobs.insert(policy.required_jobs[0].clone(), "failure".into());
    ev2.branch = "rediseno-v3".into();
    ev2.critical.push("corrupción de mundo".into());
    let b = publish::gate(&policy, &ev2, Some(&prev), &draft, &files);
    assert_eq!(b.len(), 3, "{b:?}");
    let same = release("estable", 2, "1.0.0", 1, b"inst");
    assert!(publish::gate(&policy, &ev, Some(&prev), &same, &files).iter().any(|x| x.contains("no es mayor")));
    let other = vec![(Artifact { sha256: "0".repeat(64), ..art.clone() }, String::new())];
    assert!(publish::gate(&policy, &ev, Some(&prev), &draft, &other).iter().any(|x| x.contains("no coincide")));
    let nosums = vec![(art.clone(), String::new())];
    assert!(publish::gate(&policy, &ev, Some(&prev), &draft, &nosums).iter().any(|x| x.contains("SHA256SUMS")));
}

/// Servidor HTTP mínimo: sirve `body`, respeta Range si `ranges`, y corta la primera respuesta a los
/// `cut` bytes si se pide.
async fn serve(body: Vec<u8>, ranges: bool, cut: Option<usize>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else { break };
            let n = h2.fetch_add(1, Ordering::SeqCst);
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let k = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..k]).to_lowercase();
                let start = req.lines().find_map(|l| l.strip_prefix("range: bytes=")).and_then(|r| r.trim_end_matches('-').trim().trim_end_matches('-').parse::<usize>().ok()).filter(|_| ranges).unwrap_or(0);
                let part = &body[start.min(body.len())..];
                let head = if start > 0 { format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n", part.len(), start, body.len() - 1, body.len()) } else { format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", part.len()) };
                let _ = s.write_all(head.as_bytes()).await;
                let send = if n == 0 { cut.map(|c| &part[..c.min(part.len())]).unwrap_or(part) } else { part };
                let _ = s.write_all(send).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (format!("http://{addr}/inst.exe"), hits)
}

fn artifact(body: &[u8]) -> Artifact { Artifact { name: "inst.exe".into(), size: body.len() as u64, sha256: hex::encode(Sha256::digest(body)) } }

#[tokio::test]
async fn descarga_interrumpida_se_reanuda_y_se_verifica() {
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = serve(body.clone(), true, Some(100_000)).await;
    let http = reqwest::Client::new();
    let a = artifact(&body);
    // Primera vez: se corta a mitad de camino; queda el parcial.
    let e = download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await.unwrap_err();
    assert!(matches!(e, DownloadError::Network(_)), "{e:?}");
    let part = download::part_path(tmp.path(), &a);
    let partial = std::fs::metadata(&part).unwrap().len();
    assert!(partial > 0 && partial < body.len() as u64, "parcial de {partial} bytes");
    // Segunda vez: continúa desde ahí (Range) y verifica.
    let p = download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), body);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    // Ya descargado y verificado: no vuelve a pedir nada.
    download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    host939_update::install::verify_before_launch(&p, &a).await.unwrap();
    // Si el archivo cambia en disco antes de ejecutarlo, se detecta.
    std::fs::write(&p, b"otro").unwrap();
    assert_eq!(host939_update::install::verify_before_launch(&p, &a).await, Err(DownloadError::Mismatch));
}

#[tokio::test]
async fn servidor_sin_range_paquete_manipulado_y_datos_de_mas() {
    let body: Vec<u8> = vec![7u8; 50_000];
    let http = reqwest::Client::new();
    // Sin soporte de Range: empieza de cero y termina bien.
    let tmp = tempfile::tempdir().unwrap();
    let (url, _) = serve(body.clone(), false, Some(10_000)).await;
    let a = artifact(&body);
    let _ = download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await;
    assert_eq!(std::fs::read(download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await.unwrap()).unwrap(), body);
    // Servidor que entrega otro contenido del mismo tamaño (paquete manipulado): se descarta.
    let tmp = tempfile::tempdir().unwrap();
    let evil = vec![8u8; 50_000];
    let (url, _) = serve(evil, true, None).await;
    assert_eq!(download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await, Err(DownloadError::Mismatch));
    assert!(!download::part_path(tmp.path(), &a).exists() && !tmp.path().join("inst.exe").exists(), "no queda nada del archivo malo");
    // Servidor que manda más de lo firmado (llenar el disco): se corta.
    let tmp = tempfile::tempdir().unwrap();
    let (url, _) = serve(vec![7u8; 80_000], true, None).await;
    assert_eq!(download::fetch(&http, &url, tmp.path(), &a, |_, _| {}).await, Err(DownloadError::TooLarge));
}
