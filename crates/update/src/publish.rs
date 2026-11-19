//! Lado del propietario: armar y firmar raíces y publicaciones, con los controles que bloquean una
//! publicación incompleta. Lo usa la herramienta `939publicar` (que no va en el instalador).

use crate::keys::{PublicKey, Signer};
use crate::metadata::{Artifact, Envelope, Release, RoleKeys, Root};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn root(version: u64, expires: DateTime<Utc>, root_keys: &[PublicKey], root_threshold: u32, release_keys: &[PublicKey], release_threshold: u32) -> Root {
    let mut keys = BTreeMap::new();
    for k in root_keys.iter().chain(release_keys) { keys.insert(k.key_id(), k.clone()); }
    Root {
        kind: "root".into(), version, expires, keys,
        root: RoleKeys { keyids: root_keys.iter().map(|k| k.key_id()).collect(), threshold: root_threshold },
        release: RoleKeys { keyids: release_keys.iter().map(|k| k.key_id()).collect(), threshold: release_threshold },
    }
}

pub fn sign_root(r: &Root, signers: &[&dyn Signer]) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&Envelope::sign("root", r, signers)?).map_err(|e| e.to_string())
}

pub fn sign_release(r: &Release, signers: &[&dyn Signer]) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(&Envelope::sign("release", r, signers)?).map_err(|e| e.to_string())
}

/// Resultado de las pruebas obligatorias para el commit que se quiere publicar (lo arma el propietario
/// con la información de la CI; la herramienta no confía en que «la CI pasó» sin mirar cada trabajo).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Evidence {
    pub commit: String,
    pub branch: String,
    /// Trabajo de la CI → "success" | otro.
    pub jobs: BTreeMap<String, String>,
    /// Firma Authenticode del instalador: "Valid" | "NotSigned" | …
    pub authenticode: String,
    /// Errores críticos abiertos que bloquean (lista vacía para publicar).
    #[serde(default)]
    pub critical: Vec<String>,
}

/// Reglas de publicación (en la carpeta del propietario; decisión suya).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub required_jobs: Vec<String>,
    pub branch: String,
    /// Mientras no haya certificado, false. Cuando lo haya, true y no se publica sin firma.
    pub require_authenticode: bool,
    /// Días de validez de cada publicación (después la app deja de aceptarla si no hay una nueva).
    pub release_days: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            required_jobs: [
                "Compilar instalador y portable",
                "Núcleo, P2P y grupo en Windows (servidores reales)",
                "App instalada en Windows (interfaz real)",
                "Versión de prueba (compilación de entrega)",
                "Versión de prueba instalada en Windows",
            ].iter().map(|s| s.to_string()).collect(),
            branch: "main".into(),
            require_authenticode: false,
            release_days: 120,
        }
    }
}

/// Controles antes de firmar. Devuelve la lista de bloqueos (vacía: se puede autorizar).
pub fn gate(policy: &Policy, ev: &Evidence, previous: Option<&Release>, draft: &Release, files: &[(Artifact, String)]) -> Vec<String> {
    let mut b = vec![];
    if ev.branch != policy.branch { b.push(format!("La compilación es de la rama «{}», no de «{}».", ev.branch, policy.branch)); }
    if ev.commit.len() < 40 { b.push("Falta el commit exacto de la compilación.".into()); }
    for j in &policy.required_jobs {
        match ev.jobs.get(j).map(|s| s.as_str()) {
            Some("success") => {}
            Some(other) => b.push(format!("Prueba obligatoria «{j}»: {other}.")),
            None => b.push(format!("Falta el resultado de la prueba obligatoria «{j}».")),
        }
    }
    if policy.require_authenticode && ev.authenticode != "Valid" { b.push(format!("El instalador no tiene una firma Authenticode válida ({}).", ev.authenticode)); }
    for c in &ev.critical { b.push(format!("Error crítico abierto: {c}")); }
    match semver::Version::parse(&draft.version) {
        Err(e) => b.push(format!("Versión inválida: {e}")),
        Ok(v) => {
            if (draft.channel == "estable") != v.pre.is_empty() { b.push("La versión no corresponde al canal (Estable sin «-beta», Beta con «-beta.N»).".into()); }
            if let Some(p) = previous {
                if draft.seq <= p.seq { b.push("La secuencia tiene que ser mayor que la de la publicación anterior.".into()); }
                let pv = semver::Version::parse(&p.version).ok();
                if !draft.rollback && pv.map(|pv| v <= pv).unwrap_or(false) { b.push(format!("La versión {} no es mayor que la publicada ({}).", draft.version, p.version)); }
                if draft.data_version < p.data_version && !draft.rollback { b.push("El formato de datos no puede bajar sin una vuelta atrás explícita.".into()); }
            }
        }
    }
    if draft.artifacts.is_empty() { b.push("No hay archivos para publicar.".into()); }
    for a in &draft.artifacts {
        match files.iter().find(|(f, _)| f.name == a.name) {
            None => b.push(format!("Falta el archivo {}.", a.name)),
            Some((f, _)) if f.size != a.size || f.sha256 != a.sha256 => b.push(format!("{} no coincide con lo que se va a firmar.", a.name)),
            Some((_, sums)) if !sums.contains(&a.sha256) => b.push(format!("{} no figura en el SHA256SUMS de la CI.", a.name)),
            _ => {}
        }
    }
    b
}

pub fn expires_in(days: i64) -> DateTime<Utc> { Utc::now() + Duration::days(days) }

/// Registro de publicaciones autorizadas (qué, cuándo, con qué clave y qué archivos).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: DateTime<Utc>,
    pub action: String,
    pub channel: String,
    pub version: String,
    pub seq: u64,
    pub keyid: String,
    pub commit: String,
    pub artifacts: Vec<Artifact>,
    /// SHA-256 de los metadatos firmados que se generaron.
    pub metadata_sha256: String,
}
