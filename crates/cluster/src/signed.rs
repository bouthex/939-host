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

/// JSON canónico como bytes.
pub fn canonical(v: &serde_json::Value) -> Vec<u8> { let mut out = String::new(); canon(v, &mut out); out.into_bytes() }

/// Lo que se firma: servidor, integrantes con sus roles, opciones, datos públicos (con la
/// revisión, que solo crece) y la autoridad del grupo (recuperación y sucesión).
pub fn payload(server: &str, s: &ShareConfig) -> Vec<u8> {
    let v = serde_json::json!({
        "v": 2, "server": server,
        "members": s.members.iter().map(|m| serde_json::json!({ "id": m.node_id, "name": m.name, "role": m.role, "joined": m.joined_at })).collect::<Vec<_>>(),
        "autoFailover": s.auto_failover, "handoverOnQuit": s.handover_on_quit, "card": s.card,
        "recovery": s.recovery, "recoveryPending": s.recovery_pending, "successor": s.successor, "succession": s.succession,
        "approvedMods": s.approved_mods, "modPolicy": s.mod_policy,
    });
    // Los rechazos se firman igual que las aprobaciones. Solo se agregan cuando hay alguno, para
    // que las listas firmadas antes de que existieran sigan verificando.
    let mut v = v;
    if !s.rejected_mods.is_empty() { v["rejectedMods"] = serde_json::json!(s.rejected_mods); }
    if !s.mc_ops.is_empty() { v["mcOps"] = serde_json::json!(s.mc_ops); }
    if !s.mc_ops_revoked.is_empty() { v["mcOpsRevoked"] = serde_json::json!(s.mc_ops_revoked); }
    if !s.mc_ops_base.is_empty() { v["mcOpsBase"] = serde_json::json!(s.mc_ops_base); }
    if s.mc_ops_init { v["mcOpsInit"] = serde_json::json!(true); }
    if let Some(x) = s.external_access { v["externalAccess"] = serde_json::json!(x); }
    canonical(&v)
}

pub fn owner_of(s: &ShareConfig) -> Option<String> { s.members.iter().find(|m| m.role == "propietario").map(|m| m.node_id.clone()) }

/// Revisión de la lista (sube con cada cambio firmado).
pub fn rev(s: &ShareConfig) -> u64 { s.card.as_ref().and_then(|c| c.get("rev")).and_then(|v| v.as_u64()).unwrap_or(0) }

pub fn sign(key: &SecretKey, server: &str, s: &mut ShareConfig) {
    s.sig = Some(hex::encode(key.sign(&payload(server, s)).to_bytes()));
}

/// Firma con la clave de recuperación (recuperar la propiedad o cambiar el código).
pub fn sign_recovery(rkey: &SecretKey, server: &str, s: &mut ShareConfig) {
    s.rsig = Some(hex::encode(rkey.sign(&payload(server, s)).to_bytes()));
}

fn sig_ok(signer: &str, msg: &[u8], sig: Option<&str>) -> bool {
    let (Ok(pk), Some(sig)) = (signer.parse::<PublicKey>(), sig) else { return false };
    let Ok(bytes) = hex::decode(sig) else { return false };
    let Ok(arr) = <[u8; 64]>::try_from(bytes.as_slice()) else { return false };
    pk.verify(msg, &Signature::from_bytes(&arr)).is_ok()
}

pub fn verify(server: &str, s: &ShareConfig, signer: &str) -> bool { sig_ok(signer, &payload(server, s), s.sig.as_deref()) }
fn verify_recovery(server: &str, s: &ShareConfig, rpk: &str) -> bool { sig_ok(rpk, &payload(server, s), s.rsig.as_deref()) }

/// En qué propietaria confía esta PC: la fijada al unirse o, en grupos anteriores a las firmas,
/// la propietaria que figura en su copia local (confianza en lo que ya tenía).
pub fn pin(local: &ShareConfig) -> Option<String> { local.owner_pin.clone().or_else(|| owner_of(local)) }

/* ── Código de recuperación ── */

/// Código de recuperación nuevo: 160 bits aleatorios en base32 (32 caracteres en grupos de 4).
pub fn new_recovery_code() -> String {
    let mut b = [0u8; 20];
    getrandom::getrandom(&mut b).expect("generador aleatorio del sistema");
    let s = data_encoding::BASE32_NOPAD.encode(&b);
    s.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

/// Clave ed25519 derivada del código (blake3 en modo derivación de claves, con contexto propio).
/// El código tiene 160 bits de entropía: adivinarlo no es viable, así que no hace falta una
/// derivación lenta. Acepta el código con o sin guiones, en mayúsculas o minúsculas.
pub fn recovery_key(code: &str) -> Option<SecretKey> {
    let clean: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    let bytes = data_encoding::BASE32_NOPAD.decode(clean.as_bytes()).ok()?;
    if bytes.len() != 20 { return None; }
    Some(SecretKey::from_bytes(&blake3::derive_key("939host 2026 recovery code v1", &bytes)))
}

/* ── Reclamo de la sucesora ── */

/// Reclamo firmado por la sucesora designada: «la propietaria no responde; inicio la espera».
/// Se apoya en la lista firmada por la propietaria que la designó (`base`).
pub fn claim_payload(server: &str, successor: &str, base_rev: u64) -> Vec<u8> {
    canonical(&serde_json::json!({ "v": 1, "t": "939host-succession", "server": server, "successor": successor, "rev": base_rev }))
}

pub fn make_claim(key: &SecretKey, server: &str, base: &ShareConfig) -> serde_json::Value {
    let me = key.public().to_string();
    let sig = hex::encode(key.sign(&claim_payload(server, &me, rev(base))).to_bytes());
    let mut b = base.clone();
    b.invites.clear(); b.owner_pin = None;
    serde_json::json!({ "successor": me, "rev": rev(base), "sig": sig, "base": b })
}

/// Clave única de un reclamo (para registrar cuándo lo vio esta PC por primera vez).
pub fn claim_id(claim: &serde_json::Value) -> String {
    format!("claim:{}:{}", claim["successor"].as_str().unwrap_or(""), claim["rev"].as_u64().unwrap_or(0))
}

/// Verifica un reclamo contra la propietaria en la que confía esta PC. Devuelve la lista base.
pub fn check_claim(server: &str, local: &ShareConfig, claim: &serde_json::Value) -> Result<ShareConfig, &'static str> {
    let owner = pin(local).ok_or("sin propietaria")?;
    let base: ShareConfig = serde_json::from_value(claim["base"].clone()).map_err(|_| "reclamo mal formado")?;
    let succ = claim["successor"].as_str().ok_or("reclamo mal formado")?;
    let brev = claim["rev"].as_u64().ok_or("reclamo mal formado")?;
    if owner_of(&base).as_deref() != Some(owner.as_str()) || !verify(server, &base, &owner) { return Err("la lista base no es de la propietaria"); }
    if rev(&base) != brev || brev < rev(local) { return Err("reclamo viejo: la propietaria hizo cambios después"); }
    if base.successor.as_deref() != Some(succ) || !base.member(succ).map(|m| m.role == "coanfitrion").unwrap_or(false) { return Err("esa PC no es la sucesora designada"); }
    if !sig_ok(succ, &claim_payload(server, succ, brev), claim["sig"].as_str()) { return Err("reclamo sin firma válida"); }
    Ok(base)
}

/* ── Aceptar una lista nueva ── */

/// Lo que la PC que verifica sabe además de las firmas.
pub struct Ctx<'a> {
    /// Segundos desde que esta PC vio por primera vez un pedido con espera (reclamo o cambio de código).
    pub seen_for: &'a dyn Fn(&str) -> Option<u64>,
    /// Espera obligatoria (72 h en uso real).
    pub wait_secs: u64,
    /// PCs del grupo que ya informan exactamente esta lista (revisión y propietaria).
    pub witnesses: usize,
}

fn matured(ctx: &Ctx, id: &str) -> bool { (ctx.seen_for)(id).map(|s| s >= ctx.wait_secs).unwrap_or(false) }

/// ¿Se acepta `remote` reemplazando a `local`? Reglas:
/// * la revisión tiene que ser mayor (sin rollback ni replay de listas viejas);
/// * exactamente una propietaria;
/// * firmada por la propietaria en la que confía esta PC (cambios normales o transferencia), o
/// * recuperación con el código (firma de la clave de recuperación + firma de la PC nueva), o
/// * sucesión: firmada por la sucesora designada, con su reclamo y la espera cumplida, o
/// * ya la tiene la mayoría del grupo y está firmada por su propietaria (PCs que estuvieron
///   desconectadas durante una recuperación).
pub fn accept_with(server: &str, local: &ShareConfig, remote: &ShareConfig, ctx: &Ctx) -> Result<(), &'static str> {
    let Some(p) = pin(local) else { return Err("sin propietaria de confianza") };
    if rev(remote) <= rev(local) && local.sig.is_some() { return Err("revisión vieja o repetida"); }
    if remote.members.iter().filter(|m| m.role == "propietario").count() != 1 { return Err("tiene que haber una sola propietaria"); }
    let new_owner = owner_of(remote).unwrap_or_default();
    let in_local = |n: &str| local.member(n).is_some();
    let succ_ok = |s: &ShareConfig| s.successor.as_deref().map(|x| s.member(x).map(|m| m.role == "coanfitrion").unwrap_or(false)).unwrap_or(true);
    let strict = (|| {
        if verify(server, remote, &p) {
            if !succ_ok(remote) || remote.succession.is_some() { return Err("sucesora inválida"); }
            if new_owner == p {
                if remote.recovery != local.recovery {
                    let by_code = local.recovery.as_deref().map(|rk| verify_recovery(server, remote, rk)).unwrap_or(false);
                    let first_time = local.recovery.is_none();
                    let reset = remote.recovery.is_some() && local.recovery_pending == remote.recovery && matured(ctx, &format!("reset:{}", remote.recovery.as_deref().unwrap_or("")));
                    if !(by_code || first_time || reset) { return Err("cambiar el código de recuperación exige el código actual o la espera"); }
                }
                return Ok(());
            }
            // Transferencia voluntaria: la nueva propietaria ya estaba en el grupo y arranca sin
            // código ni sucesora heredados (los de la anterior dejan de valer).
            if !in_local(&new_owner) { return Err("la nueva propietaria no es del grupo"); }
            if remote.recovery.is_some() || remote.successor.is_some() { return Err("una transferencia no hereda el código ni la sucesora"); }
            return Ok(());
        }
        // Recuperación con el código: la PC que recupera ya era del grupo, la propietaria anterior
        // queda revocada (fuera de la lista) y hay un código nuevo.
        if let Some(rk) = local.recovery.as_deref() {
            if verify_recovery(server, remote, rk) && verify(server, remote, &new_owner) {
                if !in_local(&new_owner) || new_owner == p { return Err("recuperación desde una PC que no es del grupo"); }
                if remote.member(&p).is_some() { return Err("la recuperación tiene que revocar a la propietaria anterior"); }
                if remote.recovery.is_none() || remote.recovery == local.recovery { return Err("la recuperación tiene que generar un código nuevo"); }
                if !succ_ok(remote) { return Err("sucesora inválida"); }
                return Ok(());
            }
        }
        // Sucesión con espera.
        if let Some(claim) = &remote.succession {
            let base = check_claim(server, local, claim)?;
            let succ = claim["successor"].as_str().unwrap_or_default();
            if new_owner != succ || !verify(server, remote, succ) { return Err("la sucesión la firma la sucesora"); }
            if rev(remote) != rev(&base) + 1 { return Err("sucesión sobre otra revisión"); }
            if remote.member(&p).is_some() { return Err("la sucesión tiene que revocar a la propietaria anterior"); }
            if remote.recovery != base.recovery || remote.successor.is_some() { return Err("la sucesión no cambia el código ni designa otra sucesora"); }
            if !matured(ctx, &claim_id(claim)) && ctx.witnesses < local.members.len() / 2 + 1 { return Err("todavía no pasó la espera"); }
            return Ok(());
        }
        Err("no la firmó la propietaria")
    })();
    if strict.is_ok() { return strict; }
    // Consenso: la mayoría del grupo ya la adoptó y está firmada por quien figura como propietaria.
    if ctx.witnesses >= local.members.len() / 2 + 1 && verify(server, remote, &new_owner) && in_local(&new_owner) { return Ok(()); }
    strict
}

/// Versión sin esperas ni testigos (unión, pruebas): solo cambios firmados por la propietaria,
/// transferencias y recuperación con código.
pub fn accept(server: &str, local: &ShareConfig, remote: &ShareConfig) -> bool {
    accept_with(server, local, remote, &Ctx { seen_for: &|_| None, wait_secs: u64::MAX, witnesses: 0 }).is_ok()
}

#[cfg(test)]
mod t {
    use super::*;
    use host939_core::network::Member;
    fn key(n: u8) -> SecretKey { SecretKey::from_bytes(&[n; 32]) }
    fn id(k: &SecretKey) -> String { k.public().to_string() }
    fn m(k: &SecretKey, role: &str) -> Member { Member { node_id: id(k), name: "x".into(), joined_at: "t".into(), role: role.into() } }
    fn doc(owner: &SecretKey, others: &[(&SecretKey, &str)], rev: u64) -> ShareConfig {
        let mut s = ShareConfig { enabled: true, ..Default::default() };
        s.members.push(m(owner, "propietario"));
        for (k, r) in others { s.members.push(m(k, r)); }
        s.card = Some(serde_json::json!({ "rev": rev, "name": "Z" }));
        s
    }
    fn adopt(local: &ShareConfig, remote: &ShareConfig) -> ShareConfig { let mut a = remote.clone(); a.owner_pin = owner_of(remote); let _ = local; a }
    fn ctx(seen: Option<u64>, witnesses: usize) -> impl Fn(&str) -> Option<u64> { let _ = witnesses; move |_| seen }

    #[test]
    fn only_owner_signatures_are_accepted() {
        let (a, b, c) = (key(1), key(2), key(3));
        let mut local = doc(&a, &[(&b, "coanfitrion")], 1);
        sign(&a, "srv", &mut local);
        let mut ok = doc(&a, &[(&b, "coanfitrion"), (&c, "jugador")], 2);
        sign(&a, "srv", &mut ok);
        assert!(accept("srv", &local, &ok));
        let mut evil = doc(&b, &[], 3);
        sign(&b, "srv", &mut evil);
        assert!(!accept("srv", &local, &evil), "B se declara propietaria");
        let mut tampered = ok.clone();
        tampered.members.retain(|x| x.node_id != id(&c));
        assert!(!accept("srv", &local, &tampered), "lista cambiada después de firmar");
        let mut unsigned = ok.clone(); unsigned.sig = None;
        assert!(!accept("srv", &local, &unsigned));
        let mut other = ok.clone(); sign(&a, "otro", &mut other);
        assert!(!accept("srv", &local, &other), "firma de otro servidor");
        // Coanfitriona se autodesigna sucesora: la lista no la firmó la propietaria.
        let mut self_succ = ok.clone(); self_succ.successor = Some(id(&b)); sign(&b, "srv", &mut self_succ);
        assert!(!accept("srv", &local, &self_succ));
        // La propietaria designa a un jugador como sucesor: inválido.
        let mut bad_succ = doc(&a, &[(&b, "coanfitrion"), (&c, "jugador")], 2); bad_succ.successor = Some(id(&c)); sign(&a, "srv", &mut bad_succ);
        assert!(!accept("srv", &local, &bad_succ));
    }

    #[test]
    fn old_lists_cannot_be_replayed() {
        // R5: A agrega a C como coanfitriona y después la baja a jugadora. C guardó la lista vieja
        // (firmada de verdad por A) e intenta volver a hacerla circular para recuperar el permiso.
        let (a, c) = (key(1), key(3));
        let mut v1 = doc(&a, &[(&c, "coanfitrion")], 1); sign(&a, "srv", &mut v1);
        let mut v2 = doc(&a, &[(&c, "jugador")], 2); sign(&a, "srv", &mut v2);
        let local = adopt(&v1, &v2);
        assert!(!accept("srv", &local, &v1), "rollback a una revisión anterior");
        assert!(!accept("srv", &local, &v2), "la misma revisión otra vez");
        // Misma revisión con otro contenido (aunque esté firmada por A): tampoco.
        let mut v2b = doc(&a, &[(&c, "coanfitrion")], 2); sign(&a, "srv", &mut v2b);
        assert!(!accept("srv", &local, &v2b));
        let mut v3 = doc(&a, &[], 3); sign(&a, "srv", &mut v3);
        assert!(accept("srv", &local, &v3));
    }

    #[test]
    fn transfer_moves_trust_and_drops_inherited_authority() {
        let (a, b) = (key(1), key(2));
        let mut local = doc(&a, &[(&b, "coanfitrion")], 1);
        local.owner_pin = Some(id(&a)); local.recovery = Some(id(&key(9)));
        sign(&a, "srv", &mut local);
        let mut t = doc(&b, &[(&a, "coanfitrion")], 2);
        t.recovery = local.recovery.clone();
        sign(&a, "srv", &mut t);
        assert!(!accept("srv", &local, &t), "la nueva propietaria no hereda el código de la anterior");
        t.recovery = None; sign(&a, "srv", &mut t);
        assert!(accept("srv", &local, &t));
        let adopted = adopt(&local, &t);
        let mut by_b = doc(&b, &[], 3); sign(&b, "srv", &mut by_b);
        assert!(accept("srv", &adopted, &by_b));
        let mut by_a = doc(&a, &[(&b, "coanfitrion")], 3); sign(&a, "srv", &mut by_a);
        assert!(!accept("srv", &adopted, &by_a), "la anterior ya no manda");
    }

    #[test]
    fn recovery_code_rules() {
        let (a, b, c) = (key(1), key(2), key(3));
        let code = new_recovery_code();
        assert_eq!(code.len(), 32 + 7);
        let rk = recovery_key(&code).unwrap();
        assert_eq!(recovery_key(&code.to_lowercase().replace('-', "")).unwrap().public(), rk.public(), "acepta minúsculas y sin guiones");
        assert!(recovery_key("AAAA-BBBB").is_none());
        let mut local = doc(&a, &[(&b, "coanfitrion"), (&c, "jugador")], 1);
        local.recovery = Some(id(&rk)); sign(&a, "srv", &mut local);
        let local = adopt(&local, &local);
        // B recupera la propiedad con el código: A queda revocada, código nuevo.
        let rk2 = recovery_key(&new_recovery_code()).unwrap();
        let mut rec = doc(&b, &[(&c, "jugador")], 2);
        rec.recovery = Some(id(&rk2));
        sign(&b, "srv", &mut rec); sign_recovery(&rk, "srv", &mut rec);
        assert!(accept("srv", &local, &rec));
        // Sin el código correcto (otra clave): no.
        let mut wrong = rec.clone(); sign_recovery(&rk2, "srv", &mut wrong);
        assert!(!accept("srv", &local, &wrong));
        // Tiene que revocar a la anterior.
        let mut keeps_a = doc(&b, &[(&a, "coanfitrion"), (&c, "jugador")], 2); keeps_a.recovery = Some(id(&rk2));
        sign(&b, "srv", &mut keeps_a); sign_recovery(&rk, "srv", &mut keeps_a);
        assert!(!accept("srv", &local, &keeps_a));
        // Tiene que generar un código nuevo (el viejo deja de valer).
        let mut same_code = doc(&b, &[(&c, "jugador")], 2); same_code.recovery = Some(id(&rk));
        sign(&b, "srv", &mut same_code); sign_recovery(&rk, "srv", &mut same_code);
        assert!(!accept("srv", &local, &same_code));
        // Desde una PC que no es del grupo: no.
        let x = key(7);
        let mut outsider = doc(&x, &[(&b, "coanfitrion")], 2); outsider.recovery = Some(id(&rk2));
        sign(&x, "srv", &mut outsider); sign_recovery(&rk, "srv", &mut outsider);
        assert!(!accept("srv", &local, &outsider));
        // Replay de la recuperación ya usada contra el estado nuevo: no (revisión y código viejos).
        let after = adopt(&local, &rec);
        assert!(!accept("srv", &after, &rec));
        let mut replay = rec.clone(); replay.card = Some(serde_json::json!({ "rev": 3 })); sign(&b, "srv", &mut replay); sign_recovery(&rk, "srv", &mut replay);
        assert!(accept("srv", &after, &replay), "B (nueva propietaria) puede firmar cambios normales");
        let mut steal = doc(&c, &[(&b, "jugador")], 4); steal.recovery = Some(id(&key(8)));
        sign(&c, "srv", &mut steal); sign_recovery(&rk, "srv", &mut steal);
        assert!(!accept("srv", &after, &steal), "el código viejo ya no sirve");
        // La propietaria (o quien robó su clave) no puede cambiar el código sin el código actual.
        let mut rot = doc(&b, &[(&c, "jugador")], 3); rot.recovery = Some(id(&key(8))); sign(&b, "srv", &mut rot);
        assert!(!accept("srv", &after, &rot));
        sign_recovery(&rk2, "srv", &mut rot);
        assert!(accept("srv", &after, &rot), "con el código actual sí");
    }

    #[test]
    fn lost_code_reset_waits() {
        let (a, b) = (key(1), key(2));
        let rk = recovery_key(&new_recovery_code()).unwrap();
        let newk = recovery_key(&new_recovery_code()).unwrap();
        let mut c1 = doc(&a, &[(&b, "coanfitrion")], 1); c1.recovery = Some(id(&rk)); sign(&a, "srv", &mut c1);
        let mut c2 = c1.clone(); c2.recovery_pending = Some(id(&newk)); c2.card = Some(serde_json::json!({ "rev": 2 })); sign(&a, "srv", &mut c2);
        let local = adopt(&c1, &c1);
        assert!(accept("srv", &local, &c2), "pedir el cambio sí se puede");
        let local2 = adopt(&local, &c2);
        let mut c3 = c2.clone(); c3.recovery = Some(id(&newk)); c3.recovery_pending = None; c3.card = Some(serde_json::json!({ "rev": 3 })); sign(&a, "srv", &mut c3);
        let early = ctx(Some(3600), 0);
        assert!(accept_with("srv", &local2, &c3, &Ctx { seen_for: &early, wait_secs: 72 * 3600, witnesses: 0 }).is_err(), "antes de las 72 h no");
        let late = ctx(Some(73 * 3600), 0);
        assert!(accept_with("srv", &local2, &c3, &Ctx { seen_for: &late, wait_secs: 72 * 3600, witnesses: 0 }).is_ok());
    }

    #[test]
    fn succession_waits_and_owner_can_cancel() {
        let (a, s, c, p) = (key(1), key(2), key(3), key(4));
        let mut base = doc(&a, &[(&s, "coanfitrion"), (&c, "coanfitrion"), (&p, "jugador")], 5);
        base.successor = Some(id(&s)); base.recovery = Some(id(&key(9))); sign(&a, "srv", &mut base);
        let local = adopt(&base, &base);
        let claim = make_claim(&s, "srv", &base);
        assert!(check_claim("srv", &local, &claim).is_ok());
        // Otra coanfitriona (no designada) no puede reclamar.
        assert!(check_claim("srv", &local, &make_claim(&c, "srv", &base)).is_err());
        let mut done = doc(&s, &[(&c, "coanfitrion"), (&p, "jugador")], 6);
        done.recovery = base.recovery.clone(); done.succession = Some(claim.clone());
        sign(&s, "srv", &mut done);
        let w = |secs| ctx(Some(secs), 0);
        let early = w(10 * 3600);
        assert_eq!(accept_with("srv", &local, &done, &Ctx { seen_for: &early, wait_secs: 72 * 3600, witnesses: 0 }), Err("todavía no pasó la espera"));
        let late = w(72 * 3600);
        assert!(accept_with("srv", &local, &done, &Ctx { seen_for: &late, wait_secs: 72 * 3600, witnesses: 0 }).is_ok());
        // La sucesora no puede cambiar el código de recuperación de paso.
        let mut grab = done.clone(); grab.recovery = Some(id(&key(8))); sign(&s, "srv", &mut grab);
        assert!(accept_with("srv", &local, &grab, &Ctx { seen_for: &late, wait_secs: 72 * 3600, witnesses: 0 }).is_err());
        // La propietaria cancela firmando cualquier cambio (revisión 6): el reclamo queda viejo.
        let mut cancel = base.clone(); cancel.card = Some(serde_json::json!({ "rev": 6 })); sign(&a, "srv", &mut cancel);
        assert!(accept("srv", &local, &cancel));
        let after_cancel = adopt(&local, &cancel);
        assert!(check_claim("srv", &after_cancel, &claim).is_err());
        assert!(accept_with("srv", &after_cancel, &done, &Ctx { seen_for: &late, wait_secs: 72 * 3600, witnesses: 0 }).is_err());
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
