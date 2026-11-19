//! Metadatos firmados (principios de TUF adaptados a un solo propietario y sin servidor propio).
//!
//! - `Root`: qué claves pueden firmar qué (rol `root` y rol `release`, con umbral). Cambia solo con una
//!   versión nueva firmada por el umbral de la anterior **y** de la nueva (rotación y revocación).
//! - `Release`: lo que se publica en un canal: versión, número de secuencia creciente, vencimiento y
//!   tamaño + SHA-256 de cada archivo.
//!
//! Sobre: los bytes exactos del contenido van en base64 y las firmas se hacen sobre
//! `"939host-actualizacion-v1\n" + tipo + "\n" + bytes`. Así no hay ambigüedad de formato (no hace falta
//! JSON canónico) y una firma de un tipo no sirve para otro.

use crate::keys::{PublicKey, Signer};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_METADATA_BYTES: usize = 256 * 1024;
const DOMAIN: &str = "939host-actualizacion-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Signature { pub keyid: String, pub sig: String }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    #[serde(rename = "type")]
    pub kind: String,
    /// Contenido exacto en base64.
    pub payload: String,
    pub signatures: Vec<Signature>,
}

fn pae(kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(payload.len() + 64);
    m.extend_from_slice(DOMAIN.as_bytes());
    m.push(b'\n');
    m.extend_from_slice(kind.as_bytes());
    m.push(b'\n');
    m.extend_from_slice(payload);
    m
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoleKeys { pub keyids: Vec<String>, pub threshold: u32 }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Root {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u64,
    pub expires: DateTime<Utc>,
    pub keys: BTreeMap<String, PublicKey>,
    pub root: RoleKeys,
    pub release: RoleKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Artifact {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Release {
    #[serde(rename = "type")]
    pub kind: String,
    /// "estable" | "beta"
    pub channel: String,
    /// Crece con cada publicación del canal (también al volver a firmar la misma versión).
    pub seq: u64,
    pub published: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    /// Versión semántica de la app (MAJOR.MINOR.PATCH, con `-beta.N` en Beta).
    pub version: String,
    /// Versión del formato de datos que usa esta versión (ver migraciones de datos).
    pub data_version: u32,
    /// Publicación de vuelta atrás firmada a propósito: la única forma de ofrecer una versión menor.
    #[serde(default)]
    pub rollback: bool,
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Format(String),
    WrongType,
    Signatures { needed: u32, valid: u32 },
    Expired,
    Version(String),
    Channel,
    Rollback(String),
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Format(e) => write!(f, "metadatos con formato inválido: {e}"),
            Error::WrongType => write!(f, "metadatos de otro tipo"),
            Error::Signatures { needed, valid } => write!(f, "firmas insuficientes ({valid} válidas de {needed} necesarias)"),
            Error::Expired => write!(f, "metadatos vencidos"),
            Error::Version(e) => write!(f, "versión no aceptada: {e}"),
            Error::Channel => write!(f, "la publicación es de otro canal"),
            Error::Rollback(e) => write!(f, "vuelta atrás rechazada: {e}"),
            Error::TooLarge => write!(f, "metadatos demasiado grandes"),
        }
    }
}

impl Envelope {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_METADATA_BYTES { return Err(Error::TooLarge); }
        serde_json::from_slice(bytes).map_err(|e| Error::Format(e.to_string()))
    }
    pub fn payload_bytes(&self) -> Result<Vec<u8>, Error> {
        base64::engine::general_purpose::STANDARD.decode(&self.payload).map_err(|e| Error::Format(e.to_string()))
    }
    /// Crea un sobre con el contenido serializado y la firma de cada firmante.
    pub fn sign<T: Serialize>(kind: &str, value: &T, signers: &[&dyn Signer]) -> Result<Self, String> {
        let payload = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
        let mut e = Envelope { kind: kind.into(), payload: base64::engine::general_purpose::STANDARD.encode(&payload), signatures: vec![] };
        for s in signers { e.add_signature(*s)?; }
        Ok(e)
    }
    pub fn add_signature(&mut self, s: &dyn Signer) -> Result<(), String> {
        let payload = self.payload_bytes().map_err(|e| e.to_string())?;
        let sig = s.sign(&pae(&self.kind, &payload))?;
        let keyid = s.public().key_id();
        self.signatures.retain(|x| x.keyid != keyid);
        self.signatures.push(Signature { keyid, sig: hex::encode(sig) });
        Ok(())
    }
    /// Cuenta firmas válidas de claves distintas del rol. Una misma clave repetida cuenta una vez
    /// (el error que tuvo `tough` en 2026 con los roles delegados).
    pub fn count_valid(&self, keys: &BTreeMap<String, PublicKey>, role: &RoleKeys) -> u32 {
        let Ok(payload) = self.payload_bytes() else { return 0 };
        let msg = pae(&self.kind, &payload);
        let allowed: BTreeSet<&String> = role.keyids.iter().collect();
        let mut seen = BTreeSet::new();
        for s in &self.signatures {
            if !allowed.contains(&s.keyid) || seen.contains(&s.keyid) { continue; }
            let Some(pk) = keys.get(&s.keyid) else { continue };
            // El id tiene que corresponder a la clave (no se acepta un id que apunte a otra).
            if pk.key_id() != s.keyid { continue; }
            let Ok(sig) = hex::decode(&s.sig) else { continue };
            if pk.verify(&msg, &sig) { seen.insert(&s.keyid); }
        }
        seen.len() as u32
    }
    fn check(&self, kind: &str, keys: &BTreeMap<String, PublicKey>, role: &RoleKeys) -> Result<Vec<u8>, Error> {
        if self.kind != kind { return Err(Error::WrongType); }
        if role.threshold == 0 { return Err(Error::Format("umbral 0".into())); }
        let valid = self.count_valid(keys, role);
        if valid < role.threshold { return Err(Error::Signatures { needed: role.threshold, valid }); }
        self.payload_bytes()
    }
}

impl Root {
    fn validate_shape(&self) -> Result<(), Error> {
        if self.kind != "root" { return Err(Error::WrongType); }
        for (id, k) in &self.keys {
            if &k.key_id() != id { return Err(Error::Format(format!("id de clave que no corresponde: {id}"))); }
        }
        for role in [&self.root, &self.release] {
            if role.threshold == 0 || role.threshold as usize > role.keyids.len() { return Err(Error::Format("umbral imposible".into())); }
            if role.keyids.iter().any(|k| !self.keys.contains_key(k)) { return Err(Error::Format("rol con una clave desconocida".into())); }
        }
        Ok(())
    }

    /// Lee y valida la raíz inicial que viene dentro de la app (confianza de partida).
    pub fn trusted_initial(bytes: &[u8]) -> Result<Root, Error> {
        let env = Envelope::parse(bytes)?;
        let root: Root = serde_json::from_slice(&env.payload_bytes()?).map_err(|e| Error::Format(e.to_string()))?;
        root.validate_shape()?;
        // Autofirmada por su propio umbral (detecta una raíz armada a mano o rota).
        env.check("root", &root.keys, &root.root)?;
        Ok(root)
    }

    /// Acepta la raíz `version + 1` solo si la firman el umbral de esta raíz y el de la nueva.
    /// No se mira el vencimiento de las intermedias (se pueden encadenar raíces viejas), solo de la última.
    pub fn verify_next(&self, bytes: &[u8]) -> Result<Root, Error> {
        let env = Envelope::parse(bytes)?;
        let payload = env.check("root", &self.keys, &self.root)?;
        let next: Root = serde_json::from_slice(&payload).map_err(|e| Error::Format(e.to_string()))?;
        next.validate_shape()?;
        env.check("root", &next.keys, &next.root)?;
        if next.version != self.version + 1 { return Err(Error::Version(format!("se esperaba la raíz {}, llegó la {}", self.version + 1, next.version))); }
        Ok(next)
    }

    /// Verifica una publicación con el rol `release` de esta raíz.
    pub fn verify_release(&self, bytes: &[u8], now: DateTime<Utc>) -> Result<Release, Error> {
        let env = Envelope::parse(bytes)?;
        let payload = env.check("release", &self.keys, &self.release)?;
        let r: Release = serde_json::from_slice(&payload).map_err(|e| Error::Format(e.to_string()))?;
        if r.kind != "release" { return Err(Error::WrongType); }
        if r.expires <= now { return Err(Error::Expired); }
        if r.channel != "estable" && r.channel != "beta" { return Err(Error::Channel); }
        let v = semver::Version::parse(&r.version).map_err(|e| Error::Version(e.to_string()))?;
        // Estable nunca lleva prerelease; Beta siempre (así una no se confunde con la otra).
        if (r.channel == "estable") != v.pre.is_empty() { return Err(Error::Channel); }
        if r.artifacts.is_empty() { return Err(Error::Format("sin archivos".into())); }
        for a in &r.artifacts {
            if a.sha256.len() != 64 || hex::decode(&a.sha256).is_err() { return Err(Error::Format("SHA-256 inválido".into())); }
            if a.size == 0 || a.size > 2 * 1024 * 1024 * 1024 { return Err(Error::Format("tamaño fuera de rango".into())); }
            if a.name.is_empty() || a.name.contains(['/', '\\', ':']) || a.name.starts_with('.') { return Err(Error::Format(format!("nombre de archivo no permitido: {}", a.name))); }
        }
        Ok(r)
    }
}
