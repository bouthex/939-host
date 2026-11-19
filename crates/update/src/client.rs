//! Lo que decide la app con los metadatos verificados: si hay algo para instalar, en qué canal, y qué
//! recuerda para que nadie le pueda servir algo viejo después (anti-retroceso y anti-repetición).

use crate::metadata::{Error, Release, Root};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Estado que la app guarda entre ejecuciones (en su carpeta de datos).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// Sobre de la última raíz aceptada (vacío: la que viene dentro de la app).
    #[serde(default)]
    pub root: String,
    /// Por canal: última secuencia aceptada, SHA-256 de esos metadatos y versión de raíz con la que se aceptó.
    #[serde(default)]
    pub seen: BTreeMap<String, Seen>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Seen { pub seq: u64, pub sha256: String, pub root_version: u64 }

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// No hay nada más nuevo.
    UpToDate,
    /// Hay una versión para ofrecer (o una vuelta atrás firmada).
    Offer(Release),
    /// Hay algo, pero no se puede instalar sin intervención (por ejemplo, datos de una versión más nueva).
    Blocked(String),
}

pub struct Client {
    pub root: Root,
    pub state: State,
}

const MAX_ROOT_STEPS: u64 = 64;

impl Client {
    /// `initial_root`: la raíz que viaja dentro de la app. `state`: lo guardado (puede traer una raíz posterior).
    pub fn new(initial_root: &[u8], state: State) -> Result<Self, Error> {
        let mut root = Root::trusted_initial(initial_root)?;
        if !state.root.is_empty() {
            // La raíz guardada se vuelve a encadenar desde la inicial: un archivo de estado manipulado no
            // puede introducir claves.
            let saved: Vec<String> = serde_json::from_str(&state.root).map_err(|e| Error::Format(e.to_string()))?;
            for r in saved { root = root.verify_next(r.as_bytes())?; }
        }
        Ok(Client { root, state })
    }

    /// Aplica raíces nuevas en orden (`fetch(n)` devuelve los bytes de la raíz `n`, o None si no existe).
    /// Si cambian las claves de publicación, se olvidan las secuencias vistas (recuperación ante una
    /// clave comprometida que publicó con una secuencia enorme).
    pub fn update_root(&mut self, mut fetch: impl FnMut(u64) -> Option<Vec<u8>>, now: DateTime<Utc>) -> Result<bool, Error> {
        let mut chain: Vec<String> = if self.state.root.is_empty() { vec![] } else { serde_json::from_str(&self.state.root).map_err(|e| Error::Format(e.to_string()))? };
        let start = self.root.version;
        let mut changed = false;
        while self.root.version < start + MAX_ROOT_STEPS {
            let Some(bytes) = fetch(self.root.version + 1) else { break };
            let next = self.root.verify_next(&bytes)?;
            let release_keys_changed = next.release != self.root.release || next.release.keyids.iter().any(|k| next.keys.get(k) != self.root.keys.get(k));
            if release_keys_changed { self.state.seen.clear(); }
            chain.push(String::from_utf8(bytes).map_err(|e| Error::Format(e.to_string()))?);
            self.root = next;
            changed = true;
        }
        if self.root.expires <= now { return Err(Error::Expired); }
        if changed { self.state.root = serde_json::to_string(&chain).unwrap(); }
        Ok(changed)
    }

    /// Verifica la publicación de un canal sin cambiar el estado.
    fn check_channel(&self, channel: &str, bytes: &[u8], now: DateTime<Utc>) -> Result<Release, Error> {
        let r = self.root.verify_release(bytes, now)?;
        if r.channel != channel { return Err(Error::Channel); }
        if let Some(seen) = self.state.seen.get(channel) {
            if r.seq < seen.seq { return Err(Error::Rollback(format!("secuencia {} menor que la ya vista {}", r.seq, seen.seq))); }
            if r.seq == seen.seq && sha(bytes) != seen.sha256 { return Err(Error::Rollback("misma secuencia con otro contenido".into())); }
        }
        Ok(r)
    }

    /// Decide qué ofrecer. `stable`/`beta`: metadatos descargados de cada canal (None si no se pudo).
    /// Quien eligió Estable nunca recibe Beta, aunque su metadato llegue y sea válido.
    pub fn decide(&self, user_channel: &str, stable: Option<&[u8]>, beta: Option<&[u8]>, installed: &str, data_version: u32, now: DateTime<Utc>) -> Result<Decision, Error> {
        let installed = semver::Version::parse(installed).map_err(|e| Error::Version(e.to_string()))?;
        let mut candidates = vec![];
        if let Some(b) = stable { candidates.push(self.check_channel("estable", b, now)?); }
        if user_channel == "beta" { if let Some(b) = beta { candidates.push(self.check_channel("beta", b, now)?); } }
        let mut best: Option<(semver::Version, Release)> = None;
        let mut rollback: Option<Release> = None;
        for r in candidates {
            let v = semver::Version::parse(&r.version).map_err(|e| Error::Version(e.to_string()))?;
            if r.rollback {
                if v < installed && r.channel == channel_of(&installed) { rollback = Some(r); }
                continue;
            }
            if v > installed && best.as_ref().map(|(b, _)| v > *b).unwrap_or(true) { best = Some((v, r)); }
        }
        if let Some((_, r)) = best {
            if r.data_version < data_version {
                return Ok(Decision::Blocked(format!("La versión {} usa un formato de datos anterior al de esta PC.", r.version)));
            }
            return Ok(Decision::Offer(r));
        }
        if let Some(r) = rollback {
            // Volver atrás el programa no deshace una migración de datos: solo se ofrece si la versión
            // anterior entiende los datos actuales.
            if r.data_version < data_version {
                return Ok(Decision::Blocked(format!("Volver a {} necesita restaurar el respaldo de datos previo a la migración.", r.version)));
            }
            return Ok(Decision::Offer(r));
        }
        Ok(Decision::UpToDate)
    }

    /// Recuerda lo aceptado de un canal (después de verificarlo). Llamar con los mismos bytes verificados.
    pub fn remember(&mut self, channel: &str, bytes: &[u8], now: DateTime<Utc>) -> Result<(), Error> {
        let r = self.check_channel(channel, bytes, now)?;
        self.state.seen.insert(channel.into(), Seen { seq: r.seq, sha256: sha(bytes), root_version: self.root.version });
        Ok(())
    }
}

fn channel_of(v: &semver::Version) -> &'static str { if v.pre.is_empty() { "estable" } else { "beta" } }

pub fn sha(bytes: &[u8]) -> String { hex::encode(Sha256::digest(bytes)) }
