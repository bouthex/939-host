//! Claves públicas y verificación de firmas.
//!
//! Dos esquemas, ambos con bibliotecas conocidas de RustCrypto/dalek:
//! - `ed25519`: la clave de recuperación (se genera sin conexión y se guarda fuera de la PC).
//! - `ecdsa-p256-sha256`: la clave del dispositivo del propietario, creada dentro de Windows CNG
//!   (en el TPM cuando existe) y no exportable. CNG firma P-256; Ed25519 no está en el TPM.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKey {
    /// "ed25519" | "ecdsa-p256-sha256"
    pub scheme: String,
    /// Clave pública en hexadecimal: 32 bytes (ed25519) o 65 bytes SEC1 sin comprimir (P-256).
    pub public: String,
}

impl PublicKey {
    /// Identificador estable: SHA-256 de esquema y clave. No depende de la PC ni del nombre de nadie.
    pub fn key_id(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.scheme.as_bytes());
        h.update([0u8]);
        h.update(self.public.to_ascii_lowercase().as_bytes());
        hex::encode(h.finalize())
    }

    /// Verifica `sig` sobre `msg`. Cualquier error de formato es una firma inválida.
    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        let Ok(pk) = hex::decode(&self.public) else { return false };
        match self.scheme.as_str() {
            "ed25519" => {
                let (Ok(pk), Ok(sig)) = (<[u8; 32]>::try_from(pk.as_slice()), <[u8; 64]>::try_from(sig)) else { return false };
                let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&pk) else { return false };
                // verify_strict rechaza claves débiles y firmas maleables.
                vk.verify_strict(msg, &ed25519_dalek::Signature::from_bytes(&sig)).is_ok()
            }
            "ecdsa-p256-sha256" => {
                use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
                let Ok(vk) = VerifyingKey::from_sec1_bytes(&pk) else { return false };
                let Ok(sig) = Signature::from_slice(sig) else { return false };
                // Solo firmas con s bajo: una firma no se puede transformar en otra válida.
                if sig.normalize_s() != sig { return false; }
                vk.verify(msg, &sig).is_ok()
            }
            _ => false,
        }
    }
}

/// Clave privada en memoria (solo pruebas y la clave de recuperación). Las claves del dispositivo del
/// propietario viven en Windows CNG y nunca pasan por acá.
pub enum SecretKey {
    Ed25519(ed25519_dalek::SigningKey),
    P256(p256::ecdsa::SigningKey),
}

impl SecretKey {
    pub fn generate_ed25519() -> Self {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("generador aleatorio del sistema");
        SecretKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&seed))
    }
    pub fn generate_p256() -> Self {
        loop {
            let mut b = [0u8; 32];
            getrandom::getrandom(&mut b).expect("generador aleatorio del sistema");
            if let Ok(k) = p256::ecdsa::SigningKey::from_slice(&b) { return SecretKey::P256(k); }
        }
    }
    pub fn ed25519_from_seed(seed: &[u8; 32]) -> Self { SecretKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(seed)) }
    pub fn public(&self) -> PublicKey {
        match self {
            SecretKey::Ed25519(k) => PublicKey { scheme: "ed25519".into(), public: hex::encode(k.verifying_key().to_bytes()) },
            SecretKey::P256(k) => PublicKey { scheme: "ecdsa-p256-sha256".into(), public: hex::encode(k.verifying_key().to_sec1_point(false).as_bytes()) },
        }
    }
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        match self {
            SecretKey::Ed25519(k) => { use ed25519_dalek::Signer; k.sign(msg).to_bytes().to_vec() }
            SecretKey::P256(k) => {
                use p256::ecdsa::{signature::Signer, Signature};
                let s: Signature = k.sign(msg);
                s.normalize_s().to_bytes().to_vec()
            }
        }
    }
}

/// Algo que sabe firmar con una clave cuyo lado público conocemos (CNG, memoria, etc.).
pub trait Signer {
    fn public(&self) -> PublicKey;
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, String>;
}

impl Signer for SecretKey {
    fn public(&self) -> PublicKey { SecretKey::public(self) }
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, String> { Ok(SecretKey::sign(self, msg)) }
}

/// Convierte una firma ECDSA P-256 cruda de CNG (r || s, 64 bytes) a la forma con s bajo.
pub fn p256_low_s(raw: &[u8]) -> Result<Vec<u8>, String> {
    let sig = p256::ecdsa::Signature::from_slice(raw).map_err(|e| format!("firma P-256 inválida: {e}"))?;
    Ok(sig.normalize_s().to_bytes().to_vec())
}

/// Clave pública P-256 a partir de las coordenadas X e Y (como las exporta CNG).
pub fn p256_public_from_xy(x: &[u8], y: &[u8]) -> Result<PublicKey, String> {
    if x.len() != 32 || y.len() != 32 { return Err("coordenadas P-256 de largo incorrecto".into()); }
    let mut sec1 = vec![4u8];
    sec1.extend_from_slice(x);
    sec1.extend_from_slice(y);
    p256::ecdsa::VerifyingKey::from_sec1_bytes(&sec1).map_err(|e| format!("clave P-256 inválida: {e}"))?;
    Ok(PublicKey { scheme: "ecdsa-p256-sha256".into(), public: hex::encode(sec1) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmas_de_los_dos_esquemas_y_rechazo_de_alteraciones() {
        for k in [SecretKey::generate_ed25519(), SecretKey::generate_p256()] {
            let pk = k.public();
            let sig = k.sign(b"hola");
            assert!(pk.verify(b"hola", &sig));
            assert!(!pk.verify(b"hola!", &sig), "mensaje cambiado");
            let mut bad = sig.clone();
            bad[5] ^= 1;
            assert!(!pk.verify(b"hola", &bad), "firma cambiada");
            assert!(!pk.verify(b"hola", &sig[..10]), "firma cortada");
            let other = PublicKey { scheme: if pk.scheme == "ed25519" { "ecdsa-p256-sha256".into() } else { "ed25519".into() }, public: pk.public.clone() };
            assert!(!other.verify(b"hola", &sig), "esquema cambiado");
            assert_ne!(pk.key_id(), other.key_id());
        }
    }

    #[test]
    fn p256_con_s_alto_se_rechaza() {
        let k = SecretKey::generate_p256();
        let sig = p256::ecdsa::Signature::from_slice(&k.sign(b"x")).unwrap();
        let (r, s) = sig.split_scalars();
        let neg: p256::Scalar = -*s.as_ref();
        let high = p256::ecdsa::Signature::from_scalars(r.to_bytes(), neg.to_bytes()).unwrap();
        assert_ne!(high, sig);
        assert!(!k.public().verify(b"x", &high.to_bytes()), "la versión maleable de una firma válida no se acepta");
    }
}
