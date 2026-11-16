//! Integrantes firmados. La PC propietaria firma (ed25519, con la misma clave que identifica a
//! su nodo P2P) la lista de integrantes, las opciones y los datos públicos del grupo. Cada PC
//! acepta un cambio solo si lo firmó la propietaria en la que confía; la propiedad solo cambia
//! con un documento firmado por la propietaria actual, y desde ahí se confía en la nueva.
//! Así ninguna otra PC del grupo puede agregarse, quitar a otros o declararse propietaria.

use host939_core::network::ShareConfig;
use iroh::{PublicKey, SecretKey, Signature};

/// JSON canónico (claves ordenadas), igual en todas las PCs sin depender de cómo se compile.
fn canon(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 { out.push(','); }
                out.push_str(&serde_json::to_string(k).unwrap());
                out.push(':');
                canon(&m[*k], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() { if i > 0 { out.push(','); } canon(x, out); }
            out.push(']');
        }
        x => out.push_str(&serde_json::to_string(x).unwrap()),
    }
}

/// Lo que se firma: servidor, integrantes con sus roles, opciones y datos públicos (con la revisión).
pub fn payload(server: &str, s: &ShareConfig) -> Vec<u8> {
    let v = serde_json::json!({
        "v": 1, "server": server,
        "members": s.members.iter().map(|m| serde_json::json!({ "id": m.node_id, "name": m.name, "role": m.role, "joined": m.joined_at })).collect::<Vec<_>>(),
        "autoFailover": s.auto_failover, "handoverOnQuit": s.handover_on_quit, "card": s.card,
    });
    let mut out = String::new();
    canon(&v, &mut out);
    out.into_bytes()
}

pub fn owner_of(s: &ShareConfig) -> Option<String> { s.members.iter().find(|m| m.role == "propietario").map(|m| m.node_id.clone()) }

pub fn sign(key: &SecretKey, server: &str, s: &mut ShareConfig) {
    s.sig = Some(hex::encode(key.sign(&payload(server, s)).to_bytes()));
}

pub fn verify(server: &str, s: &ShareConfig, signer: &str) -> bool {
    let (Ok(pk), Some(sig)) = (signer.parse::<PublicKey>(), s.sig.as_deref()) else { return false };
    let Ok(bytes) = hex::decode(sig) else { return false };
    let Ok(arr) = <[u8; 64]>::try_from(bytes.as_slice()) else { return false };
    pk.verify(&payload(server, s), &Signature::from_bytes(&arr)).is_ok()
}

/// En qué propietaria confía esta PC: la fijada al unirse o, en grupos anteriores a las firmas,
/// la propietaria que figura en su copia local (confianza en lo que ya tenía).
pub fn pin(local: &ShareConfig) -> Option<String> { local.owner_pin.clone().or_else(|| owner_of(local)) }

/// ¿Se acepta `remote` reemplazando a `local`? Tiene que estar firmado por la propietaria de
/// confianza y nombrar exactamente una propietaria.
pub fn accept(server: &str, local: &ShareConfig, remote: &ShareConfig) -> bool {
    let Some(p) = pin(local) else { return false };
    remote.members.iter().filter(|m| m.role == "propietario").count() == 1 && verify(server, remote, &p)
}

#[cfg(test)]
mod t {
    use super::*;
    use host939_core::network::Member;
    fn key(n: u8) -> SecretKey { SecretKey::from_bytes(&[n; 32]) }
    fn m(k: &SecretKey, role: &str) -> Member { Member { node_id: k.public().to_string(), name: "x".into(), joined_at: "t".into(), role: role.into() } }
    fn doc(owner: &SecretKey, others: &[(&SecretKey, &str)]) -> ShareConfig {
        let mut s = ShareConfig { enabled: true, ..Default::default() };
        s.members.push(m(owner, "propietario"));
        for (k, r) in others { s.members.push(m(k, r)); }
        s.card = Some(serde_json::json!({ "rev": 1, "name": "Z" }));
        s
    }
    #[test]
    fn only_owner_signatures_are_accepted() {
        let (a, b, c) = (key(1), key(2), key(3));
        let mut local = doc(&a, &[(&b, "coanfitrion")]);
        sign(&a, "srv", &mut local);
        // Cambio legítimo de la propietaria: agrega a C.
        let mut ok = doc(&a, &[(&b, "coanfitrion"), (&c, "jugador")]);
        sign(&a, "srv", &mut ok);
        assert!(accept("srv", &local, &ok));
        // B intenta declararse propietaria y quitar a A, firmando con su clave.
        let mut evil = doc(&b, &[]);
        sign(&b, "srv", &mut evil);
        assert!(!accept("srv", &local, &evil));
        // B copia la firma de A pero cambia la lista: no verifica.
        let mut tampered = ok.clone();
        tampered.members.retain(|x| x.node_id != c.public().to_string());
        assert!(!accept("srv", &local, &tampered));
        // Sin firma: no.
        let mut unsigned = ok.clone(); unsigned.sig = None;
        assert!(!accept("srv", &local, &unsigned));
        // Firma de otro servidor: no.
        let mut other = ok.clone(); sign(&a, "otro", &mut other);
        assert!(!accept("srv", &local, &other));
    }
    #[test]
    fn transfer_moves_trust() {
        let (a, b) = (key(1), key(2));
        let mut local = doc(&a, &[(&b, "coanfitrion")]);
        local.owner_pin = Some(a.public().to_string());
        // A transfiere a B: A firma el documento con B como propietaria.
        let mut t = doc(&b, &[(&a, "coanfitrion")]);
        sign(&a, "srv", &mut t);
        assert!(accept("srv", &local, &t));
        // Después de adoptarlo, la confianza pasa a B: lo firmado por B se acepta y lo de A no.
        let mut adopted = t.clone(); adopted.owner_pin = owner_of(&t);
        let mut by_b = doc(&b, &[]); sign(&b, "srv", &mut by_b);
        assert!(accept("srv", &adopted, &by_b));
        let mut by_a = doc(&a, &[(&b, "coanfitrion")]); sign(&a, "srv", &mut by_a);
        assert!(!accept("srv", &adopted, &by_a));
    }
    #[test]
    fn canonical_order_independent() {
        let a = serde_json::json!({ "b": 1, "a": [ { "y": 2, "x": 1 } ] });
        let b: serde_json::Value = serde_json::from_str(r#"{"a":[{"x":1,"y":2}],"b":1}"#).unwrap();
        let (mut s1, mut s2) = (String::new(), String::new());
        canon(&a, &mut s1); canon(&b, &mut s2);
        assert_eq!(s1, s2);
    }
}
