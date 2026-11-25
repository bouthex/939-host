//! Datos persistentes de la red del grupo: con quién se comparte cada servidor y a qué
//! servidores de amigos se unió esta PC. (El transporte P2P vive en `host939-p2p`.)

use crate::error::AppResult;
use crate::paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub node_id: String,
    pub name: String,
    pub joined_at: String,
    /// "coanfitrion" (guarda una copia y puede hostear) o "jugador" (solo juega).
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String { "jugador".into() }
fn yes() -> bool { true }

impl Member {
    pub fn can_host(&self) -> bool { self.role == "coanfitrion" || self.role == "propietario" }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub secret: String,
    pub label: String,
    pub created_at: String,
    #[serde(default = "default_role")]
    pub role: String,
    /// Vence en esta fecha (RFC 3339). Invitaciones anteriores sin fecha: 24 h desde que se crearon.
    #[serde(default)]
    pub expires_at: Option<String>,
}

impl Invite {
    /// ¿Venció? Una fecha ilegible cuenta como vencida (falla cerrado).
    pub fn expired(&self) -> bool {
        let now = chrono::Utc::now();
        let end = match &self.expires_at {
            Some(e) => chrono::DateTime::parse_from_rfc3339(e).map(|d| d.with_timezone(&chrono::Utc)).ok(),
            None => chrono::DateTime::parse_from_rfc3339(&self.created_at).map(|d| d.with_timezone(&chrono::Utc) + chrono::Duration::hours(24)).ok(),
        };
        end.map(|e| now >= e).unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareConfig {
    pub enabled: bool,
    /// Todos los integrantes del grupo de este servidor, incluida la PC que lo creó (propietario).
    pub members: Vec<Member>,
    pub invites: Vec<Invite>,
    /// Si el anfitrión se cae, las demás PCs eligen otra por mayoría y la levantan solas.
    #[serde(default = "yes")]
    pub auto_failover: bool,
    /// Al salir de la app, pasar el servidor a otra PC encendida en lugar de apagarlo.
    #[serde(default = "yes")]
    pub handover_on_quit: bool,
    /// Datos públicos del servidor (para PCs que solo juegan y no tienen copia).
    #[serde(default)]
    pub card: Option<serde_json::Value>,
    /// Firma ed25519 de la PC propietaria sobre integrantes, opciones y datos públicos.
    #[serde(default)]
    pub sig: Option<String>,
    /// Id de la PC propietaria en la que confía esta PC (se fija al unirse). Local: no se copia.
    #[serde(default)]
    pub owner_pin: Option<String>,
    /// Clave pública de recuperación (derivada del código que guarda la propietaria fuera de la PC).
    #[serde(default)]
    pub recovery: Option<String>,
    /// Cambio del código de recuperación pedido sin el código actual: vale recién tras la espera.
    #[serde(default)]
    pub recovery_pending: Option<String>,
    /// Coanfitriona designada por la propietaria para reemplazarla si deja de responder.
    #[serde(default)]
    pub successor: Option<String>,
    /// Registro verificable de una sucesión completada (reclamo firmado + lista base firmada).
    #[serde(default)]
    pub succession: Option<serde_json::Value>,
    /// Firma con la clave de recuperación (recuperar la propiedad o cambiar el código).
    #[serde(default)]
    pub rsig: Option<String>,
    /// Huellas SHA-256 de los mods aprobados por la propietaria (ver `modguard`).
    #[serde(default)]
    pub approved_mods: Vec<String>,
    /// `estricta` (por defecto) o `confiar-coanfitrionas`.
    #[serde(default)]
    pub mod_policy: Option<String>,
    /// Huellas SHA-256 de mods rechazados por la propietaria: no se cargan nunca, con ninguna
    /// política (ver `modguard`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_mods: Vec<String>,
    /// Permiso OP de Minecraft que la propietaria concedió a integrantes (por su cuenta de
    /// Minecraft). Distinto de los permisos de 939 Host: ser coanfitriona no da OP.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mc_ops: Vec<McOp>,
    /// Cuentas a las que la propietaria les quitó el OP concedido (la anfitriona se lo quita).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mc_ops_revoked: Vec<String>,
    /// Otras cuentas de Minecraft con OP autorizado por la propietaria (no atadas a una PC del
    /// grupo: su propia cuenta, amigos). Junto con `mc_ops` forman la lista de OP autorizados.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mc_ops_base: Vec<String>,
    /// La propietaria ya fijó la lista de OP autorizados (al principio, con el `ops.json` que tenía).
    /// Hasta entonces la anfitriona no quita OP que no conoce.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mc_ops_init: bool,
    /// «Permitir conexiones sin 939 Host», decidido por la propietaria (firmado). `None` = sin
    /// decidir: solo la red local, como antes de que existiera el interruptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_access: Option<bool>,
}

/// Qué tiene que hacer la anfitriona con el OP de Minecraft según la lista firmada:
/// (dar OP a, quitar OP a). `ops_now` son los nombres que tiene hoy `ops.json`.
/// - Autorizados = OP concedidos a integrantes (`mc_ops`) + `mc_ops_base`.
/// - Con la lista ya fijada (`mc_ops_init`), todo OP fuera de esa lista se quita.
/// - Antes de fijarla, solo se quita lo revocado explícitamente.
pub fn op_plan(ops_now: &[String], s: &ShareConfig) -> (Vec<String>, Vec<String>) {
    let valid = crate::players::valid_name;
    let has = |list: &[String], n: &str| list.iter().any(|x| x.eq_ignore_ascii_case(n));
    let mut auth: Vec<String> = vec![];
    for n in s.mc_ops.iter().filter(|o| s.member(&o.node_id).is_some()).map(|o| &o.mc_name).chain(s.mc_ops_base.iter()) {
        if valid(n) && !has(&auth, n) { auth.push(n.clone()); }
    }
    let to_op = auth.iter().filter(|n| !has(ops_now, n)).cloned().collect();
    let to_deop = ops_now.iter().filter(|n| valid(n) && !has(&auth, n) && (s.mc_ops_init || has(&s.mc_ops_revoked, n))).cloned().collect();
    (to_op, to_deop)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McOp { pub node_id: String, pub mc_name: String }

impl Default for ShareConfig {
    fn default() -> Self {
        ShareConfig { enabled: false, members: vec![], invites: vec![], auto_failover: true, handover_on_quit: true, card: None, sig: None, owner_pin: None, recovery: None, recovery_pending: None, successor: None, succession: None, rsig: None, approved_mods: vec![], mod_policy: None, rejected_mods: vec![], mc_ops: vec![], mc_ops_revoked: vec![], mc_ops_base: vec![], mc_ops_init: false, external_access: None }
    }
}

impl ShareConfig {
    pub fn member(&self, node: &str) -> Option<&Member> { self.members.iter().find(|m| m.node_id == node) }
}

/// Servidores en los que participa esta PC (propios compartidos, como coanfitrión o como jugador).
pub fn shared_servers() -> Vec<String> {
    std::fs::read_dir(paths::servers())
        .map(|rd| rd.flatten().filter_map(|e| {
            let id = e.file_name().to_string_lossy().to_string();
            let s = load_share(&id);
            (s.enabled && !s.members.is_empty()).then_some(id)
        }).collect())
        .unwrap_or_default()
}

/// Direcciones conocidas de otras PCs (para volver a encontrarlas aunque cambie su IP).
pub fn load_peers() -> std::collections::HashMap<String, serde_json::Value> {
    paths::read_json(&paths::root().join("peers.json")).unwrap_or_default()
}
pub fn save_peers(p: &std::collections::HashMap<String, serde_json::Value>) -> AppResult<()> {
    paths::write_json(&paths::root().join("peers.json"), p)
}

fn share_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("share.json") }
pub fn load_share(id: &str) -> ShareConfig { paths::read_json(&share_path(id)).unwrap_or_default() }
pub fn save_share(id: &str, s: &ShareConfig) -> AppResult<()> { paths::write_json(&share_path(id), s) }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteServer {
    pub key: String,
    pub host_node: String,
    /// Dirección del anfitrión serializada (clave pública + caminos conocidos).
    pub addr_json: String,
    pub server_id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub local_port: u16,
    pub joined_at: String,
}

fn remotes_path() -> std::path::PathBuf { paths::root().join("remotes.json") }
pub fn load_remotes() -> Vec<RemoteServer> { paths::read_json(&remotes_path()).unwrap_or_default() }
pub fn save_remotes(r: &[RemoteServer]) -> AppResult<()> { paths::write_json(&remotes_path(), &r.to_vec()) }

/// Puerto local fijo para un servidor de amigo: 41939, 41940, … sin repetir.
pub fn next_local_port(existing: &[RemoteServer]) -> u16 {
    (41939..42939).find(|p| !existing.iter().any(|r| r.local_port == *p)).unwrap_or(41939)
}

pub fn random_secret() -> String {
    let mut b = [0u8; 16];
    // Sin generador aleatorio del sistema no hay invitación segura posible: mejor fallar que
    // emitir un secreto predecible (antes, un error dejaba el secreto en ceros).
    getrandom::getrandom(&mut b).expect("generador aleatorio del sistema");
    hex::encode(b)
}

#[cfg(test)]
mod op_tests {
    use super::*;
    fn share(init: bool) -> ShareConfig {
        let mut s = ShareConfig { enabled: true, mc_ops_init: init, ..Default::default() };
        s.members.push(Member { node_id: "b".into(), name: "B".into(), role: "coanfitrion".into(), joined_at: String::new() });
        s.mc_ops.push(McOp { node_id: "b".into(), mc_name: "Bjug".into() });
        s.mc_ops.push(McOp { node_id: "ya-no-esta".into(), mc_name: "Viejo".into() });
        s.mc_ops_base.push("Duena".into());
        s.mc_ops_revoked.push("Quitado".into());
        s
    }
    fn v(x: &[&str]) -> Vec<String> { x.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn op_coherente_con_la_lista_firmada() {
        // Lista fijada: se da a los autorizados que falten y se quita a cualquiera de afuera.
        let (give, take) = op_plan(&v(&["duena", "Intruso", "Quitado", "Viejo"]), &share(true));
        assert_eq!(give, v(&["Bjug"]), "mayúsculas no importan; Viejo ya no es del grupo");
        assert_eq!(take, v(&["Intruso", "Quitado", "Viejo"]));
        // Antes de fijarla: solo lo revocado explícitamente (nunca se quitan OP preexistentes).
        let (_, take) = op_plan(&v(&["Intruso", "Quitado"]), &share(false));
        assert_eq!(take, v(&["Quitado"]));
        // Nombres inválidos en ops.json no generan comandos.
        let (_, take) = op_plan(&v(&["a b;stop"]), &share(true));
        assert!(take.is_empty());
    }
}
