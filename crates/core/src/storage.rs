//! Espacio de las copias de un servidor: puntos de restauración (incrementales, deduplicados) y
//! copias completas en .zip. Tope configurable, copias protegidas y verificación de integridad.
//!
//! Reglas (nunca se rompen):
//! - Nunca se borra el punto más reciente del servidor ni el último punto íntegro disponible.
//! - Nunca se borran copias manuales ni protegidas: el tope solo libera lo automático y lo viejo.
//! - Nunca se toca el mundo activo (las copias viven en su propia carpeta).
//! - La deduplicación es por bloques con su SHA-256: un bloque solo se libera cuando ningún
//!   punto lo usa (ver `snapshots::gc`).

use crate::error::{AppError, AppResult};
use crate::{backup, paths, snapshots};
use serde::Serialize;
use std::collections::BTreeSet;

fn protected_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("copias-protegidas.json") }

/// Puntos (`snap:<id>`) y copias (`zip:<archivo>`) que nunca se borran solos.
pub fn protected(id: &str) -> BTreeSet<String> { paths::read_json(&protected_path(id)).unwrap_or_default() }

pub fn set_protected(id: &str, key: &str, on: bool) -> AppResult<()> {
    if !(key.starts_with("snap:") || key.starts_with("zip:")) || key.contains(['/', '\\']) { return Err(AppError::invalid("Copia inválida.")); }
    let mut p = protected(id);
    if on { p.insert(key.into()); } else { p.remove(key); }
    paths::write_json(&protected_path(id), &p)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage { pub snapshots_bytes: u64, pub zips_bytes: u64, pub total_bytes: u64, pub cap_bytes: u64, pub snapshots: usize, pub zips: usize, pub protected: Vec<String> }

pub fn cap_bytes() -> u64 { crate::settings::load().backup_cap_gb.max(1) as u64 * 1024 * 1024 * 1024 }

pub fn usage(id: &str) -> Usage {
    let st = snapshots::stats(id);
    let zips = backup::list(id);
    let zb: u64 = zips.iter().map(|z| (z.size_mb * 1_048_576.0) as u64).sum();
    Usage { snapshots_bytes: st.store_bytes, zips_bytes: zb, total_bytes: st.store_bytes + zb, cap_bytes: cap_bytes(), snapshots: st.snapshots, zips: zips.len(), protected: protected(id).into_iter().collect() }
}

/// Hace lugar si las copias del servidor pasan el tope. Devuelve qué se borró. Orden: puntos
/// automáticos más viejos, copias locales, puntos de seguridad de más de 7 días y copias .zip de
/// seguridad de más de 7 días. Lo manual y lo protegido no se toca.
pub fn enforce_cap(id: &str, cap: u64) -> Vec<String> {
    let mut removed = vec![];
    let prot = protected(id);
    let week_ago = chrono::Local::now() - chrono::Duration::days(7);
    let old = |created: &str| chrono::DateTime::parse_from_rfc3339(created).map(|t| t < week_ago).unwrap_or(false);
    for _ in 0..500 {
        if usage(id).total_bytes <= cap { break; }
        let keep = snapshots::latest(id).map(|s| s.id);
        let mut snaps = snapshots::list(id);
        snaps.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let usable = snaps.iter().filter(|s| s.counts_for_sync()).count();
        let pick = snaps.iter().find(|s| s.kind == "auto" && Some(&s.id) != keep.as_ref() && usable > 1 && !prot.contains(&format!("snap:{}", s.id)))
            .or_else(|| snaps.iter().find(|s| s.kind == "local" && !prot.contains(&format!("snap:{}", s.id))))
            .or_else(|| snaps.iter().find(|s| s.kind == "seguridad" && old(&s.created_at) && !prot.contains(&format!("snap:{}", s.id))));
        if let Some(s) = pick {
            if snapshots::delete(id, &s.id).is_ok() { removed.push(format!("punto «{}» ({})", s.label, s.created_at.get(..16).unwrap_or(""))); continue; }
        }
        let mut zips = backup::list(id);
        zips.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        if let Some(z) = zips.iter().find(|z| z.kind == "seguridad" && old(&z.created_at) && !prot.contains(&format!("zip:{}", z.file))) {
            if backup::delete(id, &z.file).is_ok() { removed.push(format!("copia {}", z.file)); continue; }
        }
        break; // No queda nada que se pueda borrar sin romper las reglas.
    }
    removed
}

/// Verifica un punto: todos sus bloques están y coinciden con su SHA-256.
pub fn verify_snapshot(id: &str, snap: &str) -> AppResult<(usize, u64)> { snapshots::verify(id, snap) }

/// Verifica una copia .zip: se lee entera (cada archivo se comprueba con su CRC).
pub fn verify_zip(id: &str, file: &str) -> AppResult<(usize, u64)> {
    if file.contains(['/', '\\']) || !file.ends_with(".zip") { return Err(AppError::invalid("Nombre de copia inválido.")); }
    let p = paths::server_backups_dir(id).join(file);
    let f = std::fs::File::open(&p).map_err(|e| AppError::io("No se pudo abrir la copia", e))?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| AppError::new("integrity", "La copia está dañada (no se puede abrir).").with_detail(e))?;
    let (mut n, mut bytes) = (0usize, 0u64);
    let mut has_level = false;
    for i in 0..z.len() {
        let mut e = z.by_index(i).map_err(|e| AppError::new("integrity", "La copia está dañada.").with_detail(e))?;
        if e.name().map(|n| n == "world/level.dat").unwrap_or(false) { has_level = true; }
        bytes += std::io::copy(&mut e, &mut std::io::sink()).map_err(|e| AppError::new("integrity", "Un archivo de la copia está dañado (no coincide su verificación).").with_detail(e))?;
        n += 1;
    }
    if !has_level { return Err(AppError::new("integrity", "La copia no tiene el mundo (falta world/level.dat).")); }
    Ok((n, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> String {
        let _ = paths::init(std::env::temp_dir().join("939host-storage-tests"));
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        let id = format!("st{}", hex::encode(b));
        let run = paths::server_run_dir(&id);
        std::fs::create_dir_all(run.join("world/region")).unwrap();
        std::fs::write(run.join("world/level.dat"), b"nivel").unwrap();
        id
    }
    fn change(id: &str, n: u8) {
        // Contenido distinto en cada vuelta: cada punto suma bloques nuevos.
        let data: Vec<u8> = (0..300_000u32).map(|i| (i as u8).wrapping_mul(n).wrapping_add(n)).collect();
        std::fs::write(paths::server_run_dir(id).join("world/region/r.0.0.mca"), data).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    #[test]
    fn tope_libera_solo_lo_automatico_y_nunca_lo_ultimo() {
        let id = setup();
        change(&id, 1);
        let manual = snapshots::capture(&id, "a mano", "manual", "26.2").unwrap();
        let mut autos = vec![];
        for n in 2..7 { change(&id, n); autos.push(snapshots::capture(&id, "", "auto", "26.2").unwrap()); }
        set_protected(&id, &format!("snap:{}", autos[0].id), true).unwrap();
        let removed = enforce_cap(&id, 1);
        let left: Vec<String> = snapshots::list(&id).into_iter().map(|s| s.id).collect();
        assert!(left.contains(&manual.id), "lo manual no se borra");
        assert!(left.contains(&autos[0].id), "lo protegido no se borra");
        assert!(left.contains(&autos.last().unwrap().id), "el más reciente no se borra");
        assert_eq!(removed.len(), 3, "se liberan los automáticos intermedios: {removed:?}");
        // Lo que quedó sigue íntegro.
        for s in &left { verify_snapshot(&id, s).unwrap(); }
    }

    #[test]
    fn verificacion_detecta_un_bloque_danado() {
        let id = setup();
        change(&id, 9);
        let s = snapshots::capture(&id, "", "manual", "26.2").unwrap();
        assert!(verify_snapshot(&id, &s.id).is_ok());
        // Dañar un bloque guardado.
        let blobs = paths::server_dir(&id).join("restore").join("blobs");
        let sub = std::fs::read_dir(&blobs).unwrap().flatten().next().unwrap().path();
        let blob = std::fs::read_dir(&sub).unwrap().flatten().next().unwrap().path();
        std::fs::write(&blob, b"roto").unwrap();
        assert_eq!(verify_snapshot(&id, &s.id).unwrap_err().code, "integrity");
    }
}
