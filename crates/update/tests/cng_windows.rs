//! Clave del dispositivo en Windows CNG (solo Windows): no exportable, firma verificable y borrado.
//! En los equipos de la CI no hay TPM ni Windows Hello: se prueba el almacén de software sin el aviso
//! de confirmación (que bloquearía una prueba sin pantalla).

#[cfg(windows)]
#[test]
fn clave_cng_no_exportable_firma_y_se_borra() {
    use host939_update::cng::{CngKey, Store};
    use host939_update::keys::Signer;
    let name = format!("939Host-prueba-{}", std::process::id());
    let k = CngKey::create(&name, false, false).expect("crear la clave en CNG");
    println!("almacén usado: {}", k.store.label());
    assert!(!k.try_export_private(), "la clave privada no se puede exportar");
    let pk = k.public();
    assert_eq!(pk.scheme, "ecdsa-p256-sha256");
    let sig = k.sign(b"publicacion").unwrap();
    assert!(pk.verify(b"publicacion", &sig), "la firma de CNG se verifica con p256");
    assert!(!pk.verify(b"otra cosa", &sig));
    // Se puede volver a abrir por nombre y firma con la misma clave.
    let again = CngKey::open(&name, k.store).expect("reabrir");
    assert_eq!(again.public(), pk);
    drop(again);
    let store = k.store;
    k.delete().unwrap();
    assert!(CngKey::open(&name, store).is_err(), "borrada");
    let _ = Store::Software;
}
