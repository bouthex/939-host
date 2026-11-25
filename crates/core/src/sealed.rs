//! Archivos privados cifrados en reposo con DPAPI (la misma protección que la clave del nodo).
//!
//! - Solo la misma cuenta de Windows, en la misma PC, puede descifrarlos. Otro usuario de la PC,
//!   o alguien que copie el archivo a otra computadora, ve bytes ilegibles.
//! - No protege contra programas que corren con tu misma sesión (eso también puede leer lo que
//!   muestra la app).
//! - Si la clave de DPAPI se pierde (Windows reinstalado, cuenta borrada, perfil restaurado en otra
//!   PC), el contenido no se recupera: el archivo se aparta con el nombre `*.ilegible-<fecha>` (no
//!   se borra) y se empieza de nuevo.
//! - Un archivo viejo en texto plano (`.json`) se migra la primera vez que se lee: se escribe la
//!   versión cifrada, se comprueba que se puede volver a leer y recién entonces se quita el plano.

use crate::error::AppResult;
use crate::paths;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"939S1\n";

/// Resultado de leer un archivo cifrado.
pub enum Read<T> {
    Ok(T),
    /// No existe (ni cifrado ni el texto plano viejo).
    Missing,
    /// Existe pero no se pudo descifrar o no tiene el formato esperado; se apartó a `PathBuf`.
    Unreadable(PathBuf),
}

impl<T: Default> Read<T> {
    pub fn or_default(self) -> T { match self { Read::Ok(v) => v, _ => T::default() } }
}

pub fn write<T: serde::Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let json = serde_json::to_vec(value)?;
    let mut out = MAGIC.to_vec();
    out.extend(crate::identity::protect_bytes(&json)?);
    paths::write_atomic(path, &out)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    let body = bytes.strip_prefix(MAGIC)?;
    let plain = crate::identity::unprotect_bytes(body).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Lee `path` (cifrado). Si no existe y hay un `legacy` en texto plano, lo migra.
pub fn read<T: serde::de::DeserializeOwned + serde::Serialize>(path: &Path, legacy: Option<&Path>) -> Read<T> {
    match std::fs::read(path) {
        Ok(bytes) => match decode(&bytes) {
            Some(v) => Read::Ok(v),
            None => {
                let aside = path.with_extension(format!("ilegible-{}", chrono::Utc::now().format("%Y%m%d%H%M%S")));
                let _ = std::fs::rename(path, &aside);
                Read::Unreadable(aside)
            }
        },
        Err(_) => {
            let Some(old) = legacy else { return Read::Missing };
            let Ok(bytes) = std::fs::read(old) else { return Read::Missing };
            let Ok(v) = serde_json::from_slice::<T>(&bytes) else { return Read::Missing };
            // Migración: cifrado escrito y verificado antes de quitar el texto plano.
            if write(path, &v).is_ok() && std::fs::read(path).ok().and_then(|b| decode::<serde_json::Value>(&b)).is_some() {
                let _ = std::fs::remove_file(old);
            }
            Read::Ok(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cifra_migra_y_aparta_lo_ilegible() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("chat.bin");
        let old = dir.path().join("chat.json");
        // Texto plano viejo → se migra y desaparece.
        std::fs::write(&old, br#"["hola"]"#).unwrap();
        let v: Vec<String> = match read(&bin, Some(&old)) { Read::Ok(v) => v, _ => panic!("debía migrar") };
        assert_eq!(v, vec!["hola"]);
        assert!(!old.exists(), "el texto plano se quita después de verificar la copia cifrada");
        let raw = std::fs::read(&bin).unwrap();
        assert!(raw.starts_with(MAGIC));
        #[cfg(windows)]
        assert!(!raw.windows(4).any(|w| w == b"hola"), "en disco no queda el texto");
        // Se vuelve a leer.
        assert!(matches!(read::<Vec<String>>(&bin, Some(&old)), Read::Ok(v) if v == vec!["hola"]));
        // Archivo dañado o de otra cuenta → se aparta, no se borra ni se sobrescribe.
        std::fs::write(&bin, b"basura").unwrap();
        match read::<Vec<String>>(&bin, None) {
            Read::Unreadable(p) => { assert!(p.exists()); assert!(!bin.exists()); }
            _ => panic!("debía apartarlo"),
        }
        assert!(matches!(read::<Vec<String>>(&bin, None), Read::Missing));
    }
}
