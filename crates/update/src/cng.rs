//! Clave del dispositivo del propietario en Windows CNG.
//!
//! - Se crea dentro de Windows (ECDSA P-256) y **no se puede exportar**: ni el propietario, ni un
//!   programa, ni esta herramienta pueden copiar la clave privada; solo pedirle a Windows que firme.
//! - Primero se intenta el **TPM** («Microsoft Platform Crypto Provider»): la clave queda atada al chip
//!   de esa PC. Si la PC no tiene TPM 2.0 utilizable, se usa el proveedor de software de Windows
//!   (también no exportable, pero protegido solo por la cuenta de Windows).
//! - Con `confirm`, Windows muestra su propio aviso de seguridad antes de cada uso de la clave
//!   (protección fuerte de CNG). No reemplaza a un antivirus: un sistema ya comprometido puede engañar
//!   a la persona para que confirme.

use crate::keys::{p256_low_s, p256_public_from_xy, PublicKey, Signer};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Security::Cryptography::*;

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

fn check(hr: i32, what: &str) -> Result<(), String> {
    if hr == 0 { Ok(()) } else { Err(format!("{what}: error de Windows 0x{:08X}", hr as u32)) }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Store { Tpm, Software }

impl Store {
    fn provider(self) -> windows_sys::core::PCWSTR { match self { Store::Tpm => MS_PLATFORM_CRYPTO_PROVIDER, Store::Software => MS_KEY_STORAGE_PROVIDER } }
    pub fn label(self) -> &'static str { match self { Store::Tpm => "TPM de esta PC", Store::Software => "almacén de claves de Windows (sin TPM)" } }
}

pub struct CngKey { key: NCRYPT_KEY_HANDLE, prov: NCRYPT_PROV_HANDLE, pub store: Store, pub name: String, public: PublicKey }

impl Drop for CngKey {
    fn drop(&mut self) { unsafe { NCryptFreeObject(self.key); NCryptFreeObject(self.prov); } }
}

fn open_provider(store: Store) -> Result<NCRYPT_PROV_HANDLE, String> {
    let mut prov: NCRYPT_PROV_HANDLE = 0;
    check(unsafe { NCryptOpenStorageProvider(&mut prov, store.provider(), 0) }, "abrir el almacén de claves")?;
    Ok(prov)
}

fn export_public(key: NCRYPT_KEY_HANDLE) -> Result<PublicKey, String> {
    let mut len = 0u32;
    check(unsafe { NCryptExportKey(key, 0, BCRYPT_ECCPUBLIC_BLOB, std::ptr::null(), std::ptr::null_mut(), 0, &mut len, 0) }, "leer la clave pública")?;
    let mut buf = vec![0u8; len as usize];
    check(unsafe { NCryptExportKey(key, 0, BCRYPT_ECCPUBLIC_BLOB, std::ptr::null(), buf.as_mut_ptr(), len, &mut len, 0) }, "leer la clave pública")?;
    // BCRYPT_ECCKEY_BLOB { magic, cbKey } seguido de X e Y.
    if buf.len() < 8 { return Err("clave pública corta".into()); }
    let cb = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if cb != 32 || buf.len() < 8 + 2 * cb { return Err("clave pública con formato inesperado".into()); }
    p256_public_from_xy(&buf[8..8 + cb], &buf[8 + cb..8 + 2 * cb])
}

impl CngKey {
    /// Crea la clave `name`. Prueba el TPM y, si no está, el almacén de software (salvo `tpm_only`).
    pub fn create(name: &str, confirm: bool, tpm_only: bool) -> Result<CngKey, String> {
        let mut last = String::new();
        for store in [Store::Tpm, Store::Software] {
            if store == Store::Software && tpm_only { break; }
            match Self::create_in(store, name, confirm) {
                Ok(k) => return Ok(k),
                Err(e) => last = format!("{}: {e}", store.label()),
            }
        }
        Err(last)
    }

    fn create_in(store: Store, name: &str, confirm: bool) -> Result<CngKey, String> {
        let prov = open_provider(store)?;
        let wname = wide(name);
        let mut key: NCRYPT_KEY_HANDLE = 0;
        if let Err(e) = check(unsafe { NCryptCreatePersistedKey(prov, &mut key, BCRYPT_ECDSA_P256_ALGORITHM, wname.as_ptr(), 0, 0) }, "crear la clave") {
            unsafe { NCryptFreeObject(prov); }
            return Err(e);
        }
        // Política de exportación 0: no exportable (explícito, aunque es el valor por omisión).
        let zero = 0u32.to_le_bytes();
        let _ = unsafe { NCryptSetProperty(key, NCRYPT_EXPORT_POLICY_PROPERTY, zero.as_ptr(), 4, 0) };
        if confirm {
            let title = wide("939 Host — firma de publicaciones");
            let desc = wide("Windows va a pedir tu confirmación cada vez que se autorice una publicación de 939 Host.");
            let policy = NCRYPT_UI_POLICY { dwVersion: 1, dwFlags: NCRYPT_UI_PROTECT_KEY_FLAG, pszCreationTitle: title.as_ptr(), pszFriendlyName: title.as_ptr(), pszDescription: desc.as_ptr() };
            let r = unsafe { NCryptSetProperty(key, NCRYPT_UI_POLICY_PROPERTY, &policy as *const _ as *const u8, std::mem::size_of::<NCRYPT_UI_POLICY>() as u32, 0) };
            if r != 0 && store == Store::Software {
                unsafe { NCryptFreeObject(key); NCryptFreeObject(prov); }
                return check(r, "pedir confirmación en cada uso").map(|_| unreachable!());
            }
        }
        if let Err(e) = check(unsafe { NCryptFinalizeKey(key, 0) }, "guardar la clave") {
            unsafe { NCryptFreeObject(key); NCryptFreeObject(prov); }
            return Err(e);
        }
        let public = match export_public(key) { Ok(p) => p, Err(e) => { unsafe { NCryptDeleteKey(key, 0); NCryptFreeObject(prov); } return Err(e) } };
        Ok(CngKey { key, prov, store, name: name.into(), public })
    }

    /// Abre una clave ya creada en el almacén indicado.
    pub fn open(name: &str, store: Store) -> Result<CngKey, String> {
        let prov = open_provider(store)?;
        let wname = wide(name);
        let mut key: NCRYPT_KEY_HANDLE = 0;
        if let Err(e) = check(unsafe { NCryptOpenKey(prov, &mut key, wname.as_ptr(), 0, 0) }, "abrir la clave") {
            unsafe { NCryptFreeObject(prov); }
            return Err(e);
        }
        let public = match export_public(key) { Ok(p) => p, Err(e) => { unsafe { NCryptFreeObject(key); NCryptFreeObject(prov); } return Err(e) } };
        Ok(CngKey { key, prov, store, name: name.into(), public })
    }

    /// Intenta exportar la clave privada: tiene que fallar (lo usa la prueba).
    pub fn try_export_private(&self) -> bool {
        let mut len = 0u32;
        let blob = wide("ECCPRIVATEBLOB");
        unsafe { NCryptExportKey(self.key, 0, blob.as_ptr(), std::ptr::null(), std::ptr::null_mut(), 0, &mut len, 0) == 0 }
    }

    /// Borra la clave de Windows (revocación local de un dispositivo). Consume el objeto.
    pub fn delete(self) -> Result<(), String> {
        let r = unsafe { NCryptDeleteKey(self.key, 0) };
        unsafe { NCryptFreeObject(self.prov); }
        std::mem::forget(self);
        check(r, "borrar la clave")
    }
}

impl Signer for CngKey {
    fn public(&self) -> PublicKey { self.public.clone() }
    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, String> {
        let digest = Sha256::digest(msg);
        let mut out = vec![0u8; 64];
        let mut len = 0u32;
        check(unsafe { NCryptSignHash(self.key, std::ptr::null(), digest.as_ptr(), 32, out.as_mut_ptr(), 64, &mut len, 0) }, "firmar")?;
        out.truncate(len as usize);
        p256_low_s(&out)
    }
}
