//! Permisos de anfitrión firmados.
//!
//! Cada permiso (`Lease`) lo firma con ed25519 la PC que lo emite, con la misma clave que la
//! identifica en la red P2P. Una PC acepta un permiso nuevo solo si:
//!
//! * la firma es válida y corresponde a `issuer`;
//! * encadena con el permiso que ya tiene (`prev` coincide con el actual, la época sube de a uno,
//!   o es el paso de «hosteando» a «soltado» de la misma anfitriona en la misma época);
//! * quien lo emite tenía autoridad para ese tipo de cambio (ver `LeaseKind`).
//!
//! Si la PC está atrasada varias épocas, pide la cadena completa y verifica cada eslabón con las
//! mismas reglas. Un permiso viejo (época menor) nunca reemplaza a uno nuevo: no hay replay.
//! No es criptografía propia: firmas ed25519 de iroh sobre JSON canónico.

use host939_core::lease::{Lease, LeaseKind, LeaseState};
use host939_core::network::ShareConfig;
use iroh::{PublicKey, SecretKey, Signature};

/// Lo que se firma de un permiso (todo menos la firma y los votos, que llevan firma propia).
pub fn payload(server: &str, l: &Lease) -> Vec<u8> {
    let v = serde_json::json!({
        "v": 2, "t": "939host-lease", "server": server, "epoch": l.epoch, "host": l.host, "hostName": l.host_name,
        "state": l.state, "since": l.since, "required": [l.required.0, l.required.1], "kind": l.kind, "issuer": l.issuer,
        "prev": l.prev.as_ref().map(|(e, h, s)| serde_json::json!([e, h, s])),
    });
    crate::signed::canonical(&v)
}

/// Lo que firma cada PC al votar: «la anfitriona de la época `epoch` no responde; elijo a `candidate`».
pub fn vote_payload(server: &str, epoch: u64, candidate: &str) -> Vec<u8> {
    crate::signed::canonical(&serde_json::json!({ "v": 1, "t": "939host-vote", "server": server, "epoch": epoch, "candidate": candidate }))
}

pub fn sign(key: &SecretKey, server: &str, l: &mut Lease) {
    l.issuer = key.public().to_string();
    l.sig = Some(hex::encode(key.sign(&payload(server, l)).to_bytes()));
}

pub fn sign_vote(key: &SecretKey, server: &str, epoch: u64, candidate: &str) -> String {
    hex::encode(key.sign(&vote_payload(server, epoch, candidate)).to_bytes())
}

pub fn verify_sig(signer: &str, msg: &[u8], sig: &str) -> bool {
    let Ok(pk) = signer.parse::<PublicKey>() else { return false };
    let Ok(bytes) = hex::decode(sig) else { return false };
    let Ok(arr) = <[u8; 64]>::try_from(bytes.as_slice()) else { return false };
    pk.verify(msg, &Signature::from_bytes(&arr)).is_ok()
}

/// Lo que la PC que verifica sabe de la anfitriona anterior (para `Takeover`).
pub struct View<'a> {
    pub me: &'a str,
    /// ¿Esta PC tuvo contacto con `node` hace poco (dentro del plazo para darla por caída)?
    pub host_alive: &'a dyn Fn(&str) -> bool,
    /// Cuántas PCs del grupo (según su última respuesta a esta PC) ya tienen exactamente este permiso.
    pub witnesses: &'a dyn Fn(&Lease) -> usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Encadena y es válido: se adopta.
    Accept,
    /// Ya lo tenía (o es más viejo): no cambia nada.
    Stale,
    /// Hay épocas intermedias que esta PC no vio: pedir la cadena.
    Gap,
    /// Inválido: firma, autoridad o encadenamiento.
    Reject(&'static str),
}

fn majority(n: usize) -> usize { n / 2 + 1 }

/// Verifica `next` contra el permiso actual de esta PC (`cur`) y la lista firmada de integrantes.
pub fn check(server: &str, cur: Option<&Lease>, next: &Lease, share: &ShareConfig, view: &View) -> Verdict {
    if let Some(c) = cur {
        if next.epoch < c.epoch { return Verdict::Stale; }
        if next == c { return Verdict::Stale; }
        if next.epoch == c.epoch {
            // Único cambio posible en la misma época: la anfitriona suelta su permiso.
            if !(c.state == LeaseState::Hosting && next.state == LeaseState::Released && next.host == c.host) { return Verdict::Stale; }
        } else if next.epoch > c.epoch + 1 {
            return Verdict::Gap;
        }
    }
    // Sin permiso previo solo se acepta el inicial; lo demás necesita la cadena desde el origen.
    if cur.is_none() && next.kind != LeaseKind::Genesis { return Verdict::Gap; }
    // Firma de quien lo emite.
    let Some(sig) = next.sig.as_deref() else { return Verdict::Reject("permiso sin firma") };
    if next.issuer.is_empty() || !verify_sig(&next.issuer, &payload(server, next), sig) { return Verdict::Reject("firma inválida") }
    // Encadenamiento: `prev` tiene que ser exactamente el permiso que esta PC tiene.
    match (cur, &next.prev) {
        (Some(c), Some((e, h, s))) => if *e != c.epoch || *h != c.host || *s != c.state { return Verdict::Reject("no encadena con el permiso actual") },
        (Some(_), None) => return Verdict::Reject("no encadena con el permiso actual"),
        (None, _) => {}
    }
    let owner = crate::signed::pin(share);
    let can_host = |n: &str| share.member(n).map(|m| m.can_host()).unwrap_or(false);
    let prev_host = next.prev.as_ref().map(|p| p.1.as_str());
    let prev_state = next.prev.as_ref().map(|p| p.2);
    match next.kind {
        LeaseKind::Legacy => Verdict::Reject("permiso sin firma"),
        LeaseKind::Genesis => {
            if next.epoch != 1 || next.prev.is_some() || cur.is_some() { return Verdict::Reject("origen inválido"); }
            if owner.as_deref() != Some(next.issuer.as_str()) || next.host != next.issuer { return Verdict::Reject("solo la propietaria crea el permiso inicial"); }
            Verdict::Accept
        }
        LeaseKind::Release => {
            let Some(c) = cur else { return Verdict::Reject("soltar sin permiso previo") };
            if next.epoch != c.epoch || next.issuer != c.host || next.host != c.host || next.state != LeaseState::Released { return Verdict::Reject("solo la anfitriona suelta su permiso"); }
            Verdict::Accept
        }
        LeaseKind::Handover => {
            if prev_state != Some(LeaseState::Hosting) || prev_host != Some(next.issuer.as_str()) { return Verdict::Reject("solo la anfitriona actual entrega el servidor"); }
            if next.host == next.issuer || !can_host(&next.host) { return Verdict::Reject("destino que no puede hostear"); }
            if next.state != LeaseState::Hosting { return Verdict::Reject("traspaso inválido"); }
            Verdict::Accept
        }
        LeaseKind::Claim => {
            if prev_state != Some(LeaseState::Released) { return Verdict::Reject("solo se toma un permiso soltado"); }
            if next.host != next.issuer || !can_host(&next.issuer) { return Verdict::Reject("esa PC no puede hostear"); }
            Verdict::Accept
        }
        LeaseKind::Elected => {
            let Some((pe, ph, ps)) = &next.prev else { return Verdict::Reject("elección sin permiso previo") };
            if *ps != LeaseState::Hosting || next.host != next.issuer || !can_host(&next.issuer) || next.host == *ph { return Verdict::Reject("elección inválida"); }
            let msg = vote_payload(server, *pe, &next.host);
            let mut voters: Vec<&str> = vec![];
            for (voter, sig) in &next.votes {
                if voter == ph || voters.contains(&voter.as_str()) || share.member(voter).is_none() { continue; }
                if verify_sig(voter, &msg, sig) { voters.push(voter); }
            }
            // La candidata cuenta su propio voto (firmó el permiso).
            if !voters.contains(&next.host.as_str()) { voters.push(&next.host); }
            if voters.len() < majority(share.members.len()) { return Verdict::Reject("sin mayoría firmada"); }
            Verdict::Accept
        }
        LeaseKind::Takeover => {
            let Some(ph) = prev_host else { return Verdict::Reject("recuperación sin permiso previo") };
            if next.host != next.issuer || !can_host(&next.issuer) { return Verdict::Reject("esa PC no puede hostear"); }
            // La recuperación manual solo vale si el grupo también perdió a la anfitriona: quien
            // todavía la ve responder la rechaza, y la anfitriona misma solo obedece cuando la
            // mayoría del grupo ya adoptó el permiso nuevo (así una coanfitriona no apaga a una
            // anfitriona que funciona). Con dos PCs no hay testigos posibles: se acepta (riesgo
            // residual documentado; queda registrado quién lo hizo y la propietaria puede revertirlo).
            let n = share.members.len();
            let consensus = (view.witnesses)(next) >= majority(n);
            if prev_state == Some(LeaseState::Hosting) {
                if ph == view.me {
                    if n > 2 && !consensus { return Verdict::Reject("esta PC aloja el servidor y el grupo no confirmó la recuperación"); }
                } else if (view.host_alive)(ph) && !consensus {
                    return Verdict::Reject("la anfitriona anterior sigue respondiendo");
                }
            }
            Verdict::Accept
        }
        LeaseKind::Owner => {
            if owner.as_deref() != Some(next.issuer.as_str()) { return Verdict::Reject("solo la propietaria reasigna el servidor"); }
            if !can_host(&next.host) || next.state != LeaseState::Hosting { return Verdict::Reject("destino que no puede hostear"); }
            Verdict::Accept
        }
    }
}

/// Verifica una cadena (`chain`, en orden) a partir de `cur` y devuelve el último permiso válido
/// que encadena hasta `target`, o el motivo del rechazo.
pub fn check_chain(server: &str, cur: Option<&Lease>, chain: &[Lease], target: &Lease, share: &ShareConfig, view: &View) -> Result<(), &'static str> {
    let mut at = cur.cloned();
    for l in chain {
        if at.as_ref().map(|a| l.epoch < a.epoch || l == a).unwrap_or(false) { continue; }
        match check(server, at.as_ref(), l, share, view) {
            Verdict::Accept => at = Some(l.clone()),
            Verdict::Stale => continue,
            Verdict::Gap => return Err("falta un eslabón en la cadena"),
            Verdict::Reject(why) => return Err(why),
        }
        if l == target { return Ok(()); }
    }
    Err("la cadena no llega al permiso informado")
}

#[cfg(test)]
mod t {
    //! Ataques al permiso de anfitrión: cada prueba reproduce un intento y comprueba que falla.
    use super::*;
    use host939_core::network::Member;

    fn key(n: u8) -> SecretKey { SecretKey::from_bytes(&[n; 32]) }
    fn id(k: &SecretKey) -> String { k.public().to_string() }
    fn group(owner: &SecretKey, others: &[(&SecretKey, &str)]) -> ShareConfig {
        let mut s = ShareConfig { enabled: true, ..Default::default() };
        s.members.push(Member { node_id: id(owner), name: "O".into(), joined_at: "t".into(), role: "propietario".into() });
        for (k, r) in others { s.members.push(Member { node_id: id(k), name: "x".into(), joined_at: "t".into(), role: (*r).into() }); }
        s.owner_pin = Some(id(owner));
        s
    }
    fn lease(epoch: u64, host: &SecretKey, kind: LeaseKind, prev: Option<&Lease>) -> Lease {
        Lease { epoch, host: id(host), host_name: "h".into(), state: LeaseState::Hosting, since: "t".into(), required: (0, 0), kind, issuer: String::new(),
            prev: prev.map(|p| (p.epoch, p.host.clone(), p.state)), votes: vec![], sig: None }
    }
    fn signed(k: &SecretKey, mut l: Lease) -> Lease { sign(k, "srv", &mut l); l }
    fn alive_none(_: &str) -> bool { false }
    fn alive_all(_: &str) -> bool { true }
    fn zero(_: &Lease) -> usize { 0 }
    fn view<'a>(me: &'a str, alive: &'a dyn Fn(&str) -> bool, wit: &'a dyn Fn(&Lease) -> usize) -> View<'a> { View { me, host_alive: alive, witnesses: wit } }

    #[test]
    fn member_cannot_forge_or_replay_a_lease() {
        let (o, c, p) = (key(1), key(2), key(3));
        let g = group(&o, &[(&c, "coanfitrion"), (&p, "jugador")]);
        let v = view("observer", &alive_all, &zero);
        let gen = signed(&o, lease(1, &o, LeaseKind::Genesis, None));
        assert_eq!(check("srv", None, &gen, &g, &v), Verdict::Accept);
        // Un jugador se emite un permiso «entregado» a sí mismo: no es la anfitriona.
        let forged = signed(&p, lease(2, &p, LeaseKind::Handover, Some(&gen)));
        assert!(matches!(check("srv", Some(&gen), &forged, &g, &v), Verdict::Reject(_)));
        // La coanfitriona dice que la propietaria se lo entregó, firmando ella: rechazado.
        let mut fake = lease(2, &c, LeaseKind::Handover, Some(&gen));
        fake.issuer = id(&o);
        fake.sig = Some(hex::encode(c.sign(&payload("srv", &fake)).to_bytes()));
        assert_eq!(check("srv", Some(&gen), &fake, &g, &v), Verdict::Reject("firma inválida"));
        // Permiso de la propietaria modificado después de firmar (cambia la anfitriona).
        let mut tampered = signed(&o, lease(2, &c, LeaseKind::Handover, Some(&gen)));
        tampered.host = id(&p);
        assert_eq!(check("srv", Some(&gen), &tampered, &g, &v), Verdict::Reject("firma inválida"));
        // Sin firma.
        assert!(matches!(check("srv", Some(&gen), &lease(2, &c, LeaseKind::Claim, Some(&gen)), &g, &v), Verdict::Reject(_)));
        // Traspaso legítimo, y después replay del permiso viejo: no cambia nada.
        let h = signed(&o, lease(2, &c, LeaseKind::Handover, Some(&gen)));
        assert_eq!(check("srv", Some(&gen), &h, &g, &v), Verdict::Accept);
        assert_eq!(check("srv", Some(&h), &gen, &g, &v), Verdict::Stale);
        assert_eq!(check("srv", Some(&h), &h, &g, &v), Verdict::Stale);
        // Traspaso a un jugador (no puede hostear).
        let to_player = signed(&c, lease(3, &p, LeaseKind::Handover, Some(&h)));
        assert_eq!(check("srv", Some(&h), &to_player, &g, &v), Verdict::Reject("destino que no puede hostear"));
        // Encadenado a un estado que no existe (época correcta, anfitriona falsa).
        let other_prev = signed(&o, lease(2, &o, LeaseKind::Owner, Some(&gen)));
        let bad_chain = signed(&o, lease(3, &c, LeaseKind::Handover, Some(&other_prev)));
        assert_eq!(check("srv", Some(&h), &bad_chain, &g, &v), Verdict::Reject("no encadena con el permiso actual"));
        // Salto de épocas: hay que pedir la cadena.
        let far = signed(&c, lease(9, &o, LeaseKind::Handover, Some(&h)));
        assert_eq!(check("srv", Some(&h), &far, &g, &v), Verdict::Gap);
    }

    #[test]
    fn release_and_claim_rules() {
        let (o, c, c2) = (key(1), key(2), key(4));
        let g = group(&o, &[(&c, "coanfitrion"), (&c2, "coanfitrion")]);
        let v = view("x", &alive_all, &zero);
        let gen = signed(&o, lease(1, &o, LeaseKind::Genesis, None));
        // Otra PC «suelta» el permiso de la propietaria.
        let mut rel = gen.clone();
        rel.prev = Some((1, id(&o), LeaseState::Hosting)); rel.state = LeaseState::Released; rel.kind = LeaseKind::Release;
        let fake_rel = signed(&c, rel.clone());
        assert!(matches!(check("srv", Some(&gen), &fake_rel, &g, &v), Verdict::Reject(_)));
        let real_rel = signed(&o, rel);
        assert_eq!(check("srv", Some(&gen), &real_rel, &g, &v), Verdict::Accept);
        // Tomar un permiso que NO está soltado (claim sobre uno activo).
        let claim_active = signed(&c, lease(2, &c, LeaseKind::Claim, Some(&gen)));
        assert_eq!(check("srv", Some(&gen), &claim_active, &g, &v), Verdict::Reject("solo se toma un permiso soltado"));
        let claim = signed(&c, lease(2, &c, LeaseKind::Claim, Some(&real_rel)));
        assert_eq!(check("srv", Some(&real_rel), &claim, &g, &v), Verdict::Accept);
        // Dos PCs toman a la vez el mismo permiso soltado: solo la primera que se adopta vale;
        // la segunda ya no encadena (época repetida) y no reemplaza a la primera.
        let claim2 = signed(&c2, lease(2, &c2, LeaseKind::Claim, Some(&real_rel)));
        assert_eq!(check("srv", Some(&claim), &claim2, &g, &v), Verdict::Stale);
    }

    #[test]
    fn election_needs_a_signed_majority() {
        let (o, c, p1, p2) = (key(1), key(2), key(3), key(5));
        let g = group(&o, &[(&c, "coanfitrion"), (&p1, "jugador"), (&p2, "jugador")]);
        let v = view("x", &alive_all, &zero);
        let gen = signed(&o, lease(1, &o, LeaseKind::Genesis, None));
        let mut e = lease(2, &c, LeaseKind::Elected, Some(&gen));
        // Solo su propio voto: 1 de 4.
        assert_eq!(check("srv", Some(&gen), &signed(&c, e.clone()), &g, &v), Verdict::Reject("sin mayoría firmada"));
        // Votos falsificados (firmados por la candidata en nombre de otros) no cuentan.
        e.votes = vec![(id(&p1), sign_vote(&c, "srv", 1, &id(&c))), (id(&p2), sign_vote(&c, "srv", 1, &id(&c)))];
        assert_eq!(check("srv", Some(&gen), &signed(&c, e.clone()), &g, &v), Verdict::Reject("sin mayoría firmada"));
        // Votos para otra época (replay de una elección vieja) no cuentan.
        e.votes = vec![(id(&p1), sign_vote(&p1, "srv", 0, &id(&c))), (id(&p2), sign_vote(&p2, "srv", 0, &id(&c)))];
        assert_eq!(check("srv", Some(&gen), &signed(&c, e.clone()), &g, &v), Verdict::Reject("sin mayoría firmada"));
        // El mismo voto repetido cuenta una vez; el voto de la anfitriona caída no cuenta.
        e.votes = vec![(id(&p1), sign_vote(&p1, "srv", 1, &id(&c))), (id(&p1), sign_vote(&p1, "srv", 1, &id(&c))), (id(&o), sign_vote(&o, "srv", 1, &id(&c)))];
        assert_eq!(check("srv", Some(&gen), &signed(&c, e.clone()), &g, &v), Verdict::Reject("sin mayoría firmada"));
        // Mayoría real: candidata + dos jugadores = 3 de 4.
        e.votes = vec![(id(&p1), sign_vote(&p1, "srv", 1, &id(&c))), (id(&p2), sign_vote(&p2, "srv", 1, &id(&c)))];
        assert_eq!(check("srv", Some(&gen), &signed(&c, e.clone()), &g, &v), Verdict::Accept);
        // Un jugador no puede ser elegido aunque tenga votos.
        let mut ep = lease(2, &p1, LeaseKind::Elected, Some(&gen));
        ep.votes = vec![(id(&c), sign_vote(&c, "srv", 1, &id(&p1))), (id(&p2), sign_vote(&p2, "srv", 1, &id(&p1)))];
        assert_eq!(check("srv", Some(&gen), &signed(&p1, ep), &g, &v), Verdict::Reject("elección inválida"));
    }

    #[test]
    fn takeover_cannot_shut_down_a_working_host() {
        let (o, c, p) = (key(1), key(2), key(3));
        let g = group(&o, &[(&c, "coanfitrion"), (&p, "jugador")]);
        let gen = signed(&o, lease(1, &o, LeaseKind::Genesis, None));
        let t = signed(&c, lease(2, &c, LeaseKind::Takeover, Some(&gen)));
        let me = id(&o);
        // La anfitriona (propietaria) funciona y nadie más adoptó el permiso: lo rechaza.
        assert!(matches!(check("srv", Some(&gen), &t, &g, &view(&me, &alive_all, &zero)), Verdict::Reject(_)));
        // Un jugador que todavía ve a la anfitriona responder también lo rechaza.
        assert_eq!(check("srv", Some(&gen), &t, &g, &view("p", &alive_all, &zero)), Verdict::Reject("la anfitriona anterior sigue respondiendo"));
        // Si el grupo realmente la perdió: el jugador lo acepta, y la anfitriona (al volver) lo
        // acepta cuando la mayoría ya tiene el permiso nuevo.
        assert_eq!(check("srv", Some(&gen), &t, &g, &view("p", &alive_none, &zero)), Verdict::Accept);
        let two = |_: &Lease| 2usize;
        assert_eq!(check("srv", Some(&gen), &t, &g, &view(&me, &alive_all, &two)), Verdict::Accept);
        // Un jugador no puede recuperar el servidor para sí.
        let tp = signed(&p, lease(2, &p, LeaseKind::Takeover, Some(&gen)));
        assert_eq!(check("srv", Some(&gen), &tp, &g, &view("x", &alive_none, &zero)), Verdict::Reject("esa PC no puede hostear"));
        // La propietaria sí puede reasignar; nadie más puede firmar como propietaria.
        let own = signed(&o, lease(2, &o, LeaseKind::Owner, Some(&gen)));
        assert_eq!(check("srv", Some(&gen), &own, &g, &view("x", &alive_all, &zero)), Verdict::Accept);
        let fake_own = signed(&c, lease(2, &c, LeaseKind::Owner, Some(&gen)));
        assert_eq!(check("srv", Some(&gen), &fake_own, &g, &view("x", &alive_all, &zero)), Verdict::Reject("solo la propietaria reasigna el servidor"));
    }

    #[test]
    fn chain_is_verified_link_by_link() {
        let (o, c, p) = (key(1), key(2), key(3));
        let g = group(&o, &[(&c, "coanfitrion"), (&p, "jugador")]);
        let v = view("p", &alive_all, &zero);
        let gen = signed(&o, lease(1, &o, LeaseKind::Genesis, None));
        let h2 = signed(&o, lease(2, &c, LeaseKind::Handover, Some(&gen)));
        let h3 = signed(&c, lease(3, &o, LeaseKind::Handover, Some(&h2)));
        assert_eq!(check_chain("srv", Some(&gen), &[gen.clone(), h2.clone(), h3.clone()], &h3, &g, &v), Ok(()));
        // Una PC nueva sin permiso: solo con la cadena desde el origen.
        assert_eq!(check("srv", None, &h3, &g, &v), Verdict::Gap);
        assert_eq!(check_chain("srv", None, &[gen.clone(), h2.clone(), h3.clone()], &h3, &g, &v), Ok(()));
        // Eslabón intermedio falsificado por el jugador.
        let f2 = signed(&p, lease(2, &p, LeaseKind::Handover, Some(&gen)));
        let f3 = signed(&p, lease(3, &c, LeaseKind::Handover, Some(&f2)));
        assert!(check_chain("srv", Some(&gen), &[f2, f3.clone()], &f3, &g, &v).is_err());
        // Cadena incompleta.
        assert!(check_chain("srv", Some(&gen), &[h3.clone()], &h3, &g, &v).is_err());
    }
}

#[cfg(test)]
mod costo {
    /// Costo de verificar un permiso y una lista firmada (se corre con --ignored --nocapture).
    #[test]
    #[ignore]
    fn medir_verificacion() {
        use super::*;
        use host939_core::network::Member;
        let k = |n: u8| SecretKey::from_bytes(&[n; 32]);
        let (o, c) = (k(1), k(2));
        let mut g = ShareConfig { enabled: true, ..Default::default() };
        for (key, r) in [(&o, "propietario"), (&c, "coanfitrion")] { g.members.push(Member { node_id: key.public().to_string(), name: "x".into(), joined_at: "t".into(), role: r.into() }); }
        for i in 0..18u8 { g.members.push(Member { node_id: k(10 + i).public().to_string(), name: "x".into(), joined_at: "t".into(), role: "jugador".into() }); }
        g.owner_pin = Some(o.public().to_string());
        g.card = Some(serde_json::json!({ "rev": 1 }));
        let mut gen = Lease { epoch: 1, host: o.public().to_string(), host_name: "h".into(), state: LeaseState::Hosting, since: "t".into(), required: (0, 0), kind: LeaseKind::Genesis, issuer: String::new(), prev: None, votes: vec![], sig: None };
        sign(&o, "s", &mut gen);
        let mut h = Lease { epoch: 2, host: c.public().to_string(), kind: LeaseKind::Handover, prev: Some((1, o.public().to_string(), LeaseState::Hosting)), ..gen.clone() };
        sign(&o, "s", &mut h);
        let alive = |_: &str| true; let wit = |_: &Lease| 0usize;
        let v = View { me: "x", host_alive: &alive, witnesses: &wit };
        let n = 2000;
        let t = std::time::Instant::now();
        for _ in 0..n { assert_eq!(check("s", Some(&gen), &h, &g, &v), Verdict::Accept); }
        let lease_us = t.elapsed().as_micros() as f64 / n as f64;
        let mut g2 = g.clone(); g2.card = Some(serde_json::json!({ "rev": 2 })); crate::signed::sign(&o, "s", &mut g2);
        let t = std::time::Instant::now();
        for _ in 0..n { assert!(crate::signed::accept("s", &g, &g2)); }
        let list_us = t.elapsed().as_micros() as f64 / n as f64;
        println!("verificar un permiso: {lease_us:.1} µs · verificar una lista de 20 PCs: {list_us:.1} µs");
    }
}
