//! La distribución que viaja dentro de la app y el modelo de publicación desde GitHub.

use chrono::{Duration, Utc};
use host939_update::client::{Client, Decision, State};
use host939_update::keys::SecretKey;
use host939_update::metadata::{Artifact, Release, Root};
use host939_update::publish;

fn release(version: &str, seq: u64) -> Release {
    Release { kind: "release".into(), channel: "estable".into(), seq, published: Utc::now(), expires: Utc::now() + Duration::days(730), version: version.into(), data_version: 1, rollback: false, artifacts: vec![Artifact { name: format!("939Host-{version}-instalador.exe"), size: 10, sha256: "ab".repeat(32) }], notes: String::new() }
}

#[test]
fn la_raiz_oficial_es_valida_publica_y_apunta_a_github_por_https() {
    let d: serde_json::Value = serde_json::from_str(include_str!("../../../src-tauri/distribucion.json")).unwrap();
    let url = d["url"].as_str().expect("la dirección está configurada");
    assert_eq!(url, "https://github.com/bouthex/939-host/releases/latest/download", "fuente oficial única: las releases de GitHub del proyecto");
    let raiz = serde_json::to_vec(&d["raiz"]).unwrap();
    let root = Root::trusted_initial(&raiz).expect("raíz autofirmada válida");
    assert_eq!(root.version, 1);
    assert!(root.expires > Utc::now() + Duration::days(365 * 5), "la raíz no vence pronto");
    // La clave de publicación (GitHub) no puede firmar raíces: rotar o revocar exige la de recuperación.
    assert!(root.release.keyids.iter().all(|k| !root.root.keyids.contains(k)));
    // Solo claves públicas: cada clave tiene esquema y parte pública, nada más.
    for k in root.keys.values() { assert_eq!(k.public.len(), 64, "ed25519 pública"); }
    assert_eq!(d.as_object().unwrap().keys().filter(|k| !k.starts_with('_')).count(), 2, "solo «raiz» y «url»");
    // Es la misma raíz publicada junto a las releases.
    let publicada: serde_json::Value = serde_json::from_str(include_str!("../../../actualizaciones/raiz-1.json")).unwrap();
    assert_eq!(publicada, d["raiz"]);
}

#[test]
fn versiones_se_comparan_por_numero_no_por_texto() {
    let recovery = SecretKey::generate_ed25519();
    let ci = SecretKey::generate_ed25519();
    let root = publish::sign_root(&publish::root(1, publish::expires_in(3650), &[recovery.public()], 1, &[ci.public()], 1), &[&recovery]).unwrap();
    let c = Client::new(&root, State::default()).unwrap();
    let r = publish::sign_release(&release("1.0.10", 2), &[&ci]).unwrap();
    assert!(matches!(c.decide("estable", Some(&r), None, "1.0.2", 1, Utc::now()).unwrap(), Decision::Offer(_)), "1.0.10 es más nueva que 1.0.2");
    let r = publish::sign_release(&release("1.0.2", 3), &[&ci]).unwrap();
    assert_eq!(c.decide("estable", Some(&r), None, "1.0.10", 1, Utc::now()).unwrap(), Decision::UpToDate, "1.0.2 no reemplaza a 1.0.10");
    assert_eq!(c.decide("estable", Some(&r), None, "1.0.2", 1, Utc::now()).unwrap(), Decision::UpToDate, "misma versión: nada que descargar");
    // Sin metadatos (la release más reciente es anterior al sistema de actualizaciones): al día.
    assert_eq!(c.decide("estable", None, None, "1.1.0", 1, Utc::now()).unwrap(), Decision::UpToDate);
    // Firmado con otra clave (por ejemplo, una compilación de prueba): rechazado.
    let otra = SecretKey::generate_ed25519();
    let r = publish::sign_release(&release("9.0.0", 4), &[&otra]).unwrap();
    assert!(c.decide("estable", Some(&r), None, "1.1.0", 1, Utc::now()).is_err());
}
