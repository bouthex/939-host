//! Permiso de anfitrión ("lease"): la regla que impide que dos PCs escriban el mismo mundo.
//!
//! Cada servidor lógico tiene un `lease.json` con una época (número que solo crece) y la PC
//! que tiene permiso para hostear. Una PC solo puede encender el servidor si el permiso es suyo
//! o si nadie lo tiene tomado. Para tomarlo, sube la época; cualquier PC que vea una época mayor
//! que la suya obedece a la nueva y, si estaba hosteando con una época vieja, se detiene
//! ("se aísla") y su copia queda apartada como punto de seguridad: nunca se mezcla.

use crate::error::{AppError, AppResult};
use crate::paths;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

static LOCAL_NODE: OnceCell<String> = OnceCell::new();

/// Registra el id de esta PC (se llama una vez al iniciar).
pub fn set_local_node(id: &str) { let _ = LOCAL_NODE.set(id.to_string()); }
pub fn local_node() -> Option<String> { LOCAL_NODE.get().cloned() }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LeaseState {
    /// La PC `host` tiene el servidor (encendido o por encenderse).
    #[default]
    Hosting,
    /// La PC `host` lo soltó de forma ordenada: cualquiera puede tomarlo.
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub epoch: u64,
    pub host: String,
    #[serde(default)]
    pub host_name: String,
    #[serde(default)]
    pub state: LeaseState,
    pub since: String,
    /// Al soltarlo: el punto mínimo (época, orden) que hay que tener para poder tomarlo sin
    /// perder progreso. Lo fija quien suelta, con su último punto.
    #[serde(default)]
    pub required: (u64, u64),
    /// Cómo se obtuvo este permiso (ver `LeaseKind`). Vacío solo en permisos anteriores a las firmas.
    #[serde(default)]
    pub kind: LeaseKind,
    /// PC que emitió y firmó este permiso.
    #[serde(default)]
    pub issuer: String,
    /// Permiso al que reemplaza (época, anfitriona, estado): la firma lo cubre, así que no se
    /// puede encadenar a un estado que no existió.
    #[serde(default)]
    pub prev: Option<(u64, String, LeaseState)>,
    /// Votos firmados de la elección (solo `Elected`): (PC que votó, firma).
    #[serde(default)]
    pub votes: Vec<(String, String)>,
    /// Firma ed25519 de `issuer` sobre todo lo anterior (salvo los votos, que se firman aparte).
    #[serde(default)]
    pub sig: Option<String>,
}

/// Origen de un permiso. Cada tipo tiene su regla de autoridad (se verifica en el grupo):
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LeaseKind {
    /// Anterior a las firmas (solo se acepta el que ya estaba en disco).
    #[default]
    Legacy,
    /// Época 1: lo emite la propietaria al compartir.
    Genesis,
    /// La anfitriona actual se lo entrega a otra PC.
    Handover,
    /// La anfitriona actual lo suelta (misma época).
    Release,
    /// Una PC que puede hostear toma un permiso soltado.
    Claim,
    /// Elegida por mayoría con votos firmados porque la anfitriona dejó de responder.
    Elected,
    /// «Recuperar aquí» de una coanfitriona: solo lo acepta quien tampoco tiene contacto con la
    /// anfitriona anterior; una anfitriona que funciona lo rechaza.
    Takeover,
    /// La propietaria lo reasigna (control total del grupo).
    Owner,
}

impl Lease {
    pub fn held_by(&self, node: &str) -> bool { self.state == LeaseState::Hosting && self.host == node }
    pub fn is_free(&self) -> bool { self.state == LeaseState::Released }
}

fn path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("lease.json") }

pub fn read(id: &str) -> Option<Lease> { paths::read_json(&path(id)).ok() }

fn write(id: &str, l: &Lease) -> AppResult<()> { paths::write_json(&path(id), l) }

/// ¿Puede esta PC encender el servidor? Servidores que nunca se compartieron no tienen permiso
/// y siempre se pueden encender.
pub fn can_start(id: &str) -> AppResult<()> {
    let Some(l) = read(id) else { return Ok(()) };
    if l.state == LeaseState::Released { return Ok(()); }
    match local_node() {
        Some(me) if l.host == me => Ok(()),
        None => Ok(()),
        _ => Err(AppError::busy(format!(
            "Este servidor lo está hosteando otra PC del grupo{}. Pedile que te lo pase o esperá a que lo suelte.",
            if l.host_name.is_empty() { String::new() } else { format!(" ({})", l.host_name) }
        ))),
    }
}

/// Toma el permiso subiendo la época. `expected` es la época que esta PC conoce: si en disco
/// hay una distinta (alguien más avanzó), no se toma.
pub fn claim(id: &str, expected: u64, host_name: &str) -> AppResult<Lease> {
    let me = local_node().ok_or_else(|| AppError::new("lease", "Falta la identidad de esta PC."))?;
    let cur = read(id);
    let cur_epoch = cur.as_ref().map(|l| l.epoch).unwrap_or(0);
    if cur_epoch != expected {
        return Err(AppError::busy("El estado del grupo cambió mientras tanto; se volvió a consultar."));
    }
    let prev = cur.as_ref().map(|c| (c.epoch, c.host.clone(), c.state));
    let kind = match &cur { None => LeaseKind::Genesis, Some(c) if c.state == LeaseState::Released => LeaseKind::Claim, Some(_) => LeaseKind::Takeover };
    let l = Lease { epoch: cur_epoch + 1, host: me.clone(), host_name: host_name.into(), state: LeaseState::Hosting, since: chrono::Local::now().to_rfc3339(), required: (0, 0), kind, issuer: me, prev, votes: vec![], sig: None };
    write(id, &l)?;
    Ok(l)
}

/// Permiso nuevo para esta PC sin escribirlo (lo firma y lo guarda el grupo con `store`).
pub fn draft_claim(id: &str, expected: u64, host_name: &str, kind: LeaseKind) -> AppResult<Lease> {
    let me = local_node().ok_or_else(|| AppError::new("lease", "Falta la identidad de esta PC."))?;
    let cur = read(id);
    let cur_epoch = cur.as_ref().map(|l| l.epoch).unwrap_or(0);
    if cur_epoch != expected {
        return Err(AppError::busy("El estado del grupo cambió mientras tanto; se volvió a consultar."));
    }
    let prev = cur.as_ref().map(|c| (c.epoch, c.host.clone(), c.state));
    Ok(Lease { epoch: cur_epoch + 1, host: me.clone(), host_name: host_name.into(), state: LeaseState::Hosting, since: chrono::Local::now().to_rfc3339(), required: (0, 0), kind, issuer: me, prev, votes: vec![], sig: None })
}

/// Guarda un permiso ya verificado (o firmado por esta PC) y lo agrega al historial.
pub fn store(id: &str, l: &Lease) -> AppResult<()> {
    write(id, l)?;
    let mut log = history(id);
    if log.last() != Some(l) { log.push(l.clone()); }
    let n = log.len();
    if n > HISTORY_MAX { log.drain(0..n - HISTORY_MAX); }
    paths::write_json(&paths::server_dir(id).join("lease-log.json"), &log)
}

/// Permisos verificados en orden (para que una PC atrasada pueda comprobar la cadena completa).
pub fn history(id: &str) -> Vec<Lease> { paths::read_json(&paths::server_dir(id).join("lease-log.json")).unwrap_or_default() }
const HISTORY_MAX: usize = 4096;

/// Suelta el permiso de forma ordenada (después de apagar y de dejar el último punto).
pub fn release(id: &str, required: (u64, u64)) -> AppResult<Option<Lease>> {
    let Some(mut l) = read(id) else { return Ok(None) };
    if Some(&l.host) != local_node().as_ref() { return Ok(Some(l)); }
    l.prev = Some((l.epoch, l.host.clone(), l.state));
    l.state = LeaseState::Released;
    l.required = required;
    l.since = chrono::Local::now().to_rfc3339();
    l.kind = LeaseKind::Release;
    l.issuer = l.host.clone();
    l.votes.clear();
    l.sig = None;
    write(id, &l)?;
    Ok(Some(l))
}

/// Entrega directa a otra PC (traspaso planificado): misma operación que un `claim` hecho por ella.
/// Permiso que se le entregaría a otra PC (traspaso planificado). No se escribe: quien lo
/// recibe lo adopta y recién entonces quien lo entrega lo adopta también.
pub fn next_for(id: &str, to: &str, to_name: &str) -> Lease {
    let cur = read(id);
    let cur_epoch = cur.as_ref().map(|l| l.epoch).unwrap_or(0);
    let prev = cur.as_ref().map(|c| (c.epoch, c.host.clone(), c.state));
    Lease { epoch: cur_epoch + 1, host: to.into(), host_name: to_name.into(), state: LeaseState::Hosting, since: chrono::Local::now().to_rfc3339(), required: (0, 0), kind: LeaseKind::Handover, issuer: local_node().unwrap_or_default(), prev, votes: vec![], sig: None }
}

/// Toma forzada ("Recuperar aquí"): sube la época sin importar quién lo tenía. Si el anfitrión
/// anterior vuelve, ve la época mayor, se detiene y aparta su copia.
pub fn force_claim(id: &str, host_name: &str) -> AppResult<Lease> {
    let cur = read(id).map(|l| l.epoch).unwrap_or(0);
    claim(id, cur, host_name)
}

/// Adopta el permiso que informa otra PC si es más nuevo. Devuelve true si cambió.
/// Con la misma época solo se acepta el paso de "hosteando" a "soltado" por parte del mismo anfitrión.
pub fn adopt(id: &str, remote: &Lease) -> AppResult<bool> {
    let cur = read(id);
    let newer = match &cur {
        None => true,
        Some(c) => remote.epoch > c.epoch
            || (remote.epoch == c.epoch && c.host == remote.host && c.state == LeaseState::Hosting && remote.state == LeaseState::Released),
    };
    if newer { store(id, remote)?; }
    Ok(newer)
}

/// Solo para el primer uso compartido: si no hay permiso, lo crea a nombre de esta PC (época 1).
pub fn ensure(id: &str, host_name: &str) -> AppResult<Lease> {
    if let Some(l) = read(id) { return Ok(l); }
    claim(id, 0, host_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> String {
        let dir = std::env::temp_dir().join(format!("h939-lease-{}", std::process::id()));
        let _ = paths::init(dir);
        set_local_node("nodo-a");
        let id = format!("srv{}", rand_suffix());
        std::fs::create_dir_all(paths::server_dir(&id)).unwrap();
        id
    }
    fn blank() -> Lease { Lease { epoch: 0, host: String::new(), host_name: String::new(), state: LeaseState::Hosting, since: String::new(), required: (0, 0), kind: LeaseKind::Legacy, issuer: String::new(), prev: None, votes: vec![], sig: None } }
    fn rand_suffix() -> u64 { let mut b = [0u8; 8]; getrandom::getrandom(&mut b).unwrap(); u64::from_le_bytes(b) }

    #[test]
    fn epochs_only_grow_and_others_cannot_start() {
        let id = setup();
        assert!(can_start(&id).is_ok(), "sin permiso se puede encender");
        let l = ensure(&id, "PC A").unwrap();
        assert_eq!(l.epoch, 1);
        assert!(can_start(&id).is_ok());
        // Otra PC toma el permiso (llega por la red con época mayor)
        let remote = Lease { epoch: 2, host: "nodo-b".into(), host_name: "PC B".into(), state: LeaseState::Hosting, since: "x".into(), required: (0, 0), ..blank() };
        assert!(adopt(&id, &remote).unwrap());
        assert!(can_start(&id).is_err(), "con el permiso en otra PC no se enciende");
        // Un permiso viejo no pisa al nuevo
        let stale = Lease { epoch: 1, host: "nodo-a".into(), host_name: "".into(), state: LeaseState::Hosting, since: "x".into(), required: (0, 0), ..blank() };
        assert!(!adopt(&id, &stale).unwrap());
        assert_eq!(read(&id).unwrap().host, "nodo-b");
        // Claim con época desactualizada falla
        assert!(claim(&id, 1, "PC A").is_err());
        // B suelta: misma época, estado soltado → se acepta y A puede encender
        let rel = Lease { state: LeaseState::Released, ..remote.clone() };
        assert!(adopt(&id, &rel).unwrap());
        assert!(can_start(&id).is_ok());
        let mine = claim(&id, 2, "PC A").unwrap();
        assert_eq!(mine.epoch, 3);
        assert!(mine.held_by("nodo-a"));
        // Un "soltado" de otro anfitrión con la misma época no se acepta
        let fake = Lease { epoch: 3, host: "nodo-b".into(), host_name: "".into(), state: LeaseState::Released, since: "x".into(), required: (0, 0), ..blank() };
        assert!(!adopt(&id, &fake).unwrap());
    }
}
