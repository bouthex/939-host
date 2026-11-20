//! Versión del formato de los datos de 939 Host y migraciones seguras.
//!
//! - `version-datos` en la carpeta de datos dice en qué formato están configuración, grupos, amigos e
//!   identidad (no los mundos: Minecraft migra los suyos y 939 Host nunca los reescribe).
//! - Antes de migrar se copia todo lo que no es mundo, copia de seguridad, caché ni Java a
//!   `respaldos-datos/v<N>-<fecha>/`; después de migrar se valida; si algo falla se restaura el respaldo
//!   y la app sigue con el formato anterior.
//! - Si los datos son de una versión **más nueva** (alguien instaló una versión vieja encima), esta
//!   versión no toca nada: lo correcto es volver a la nueva o restaurar el respaldo de la migración.
//!   Volver atrás el programa no deshace una migración de datos.

use crate::paths;
use std::path::{Path, PathBuf};

/// Formato de datos que entiende esta versión.
pub const DATA_VERSION: u32 = 1;

/// Paso de migración: lleva los datos de `to - 1` a `to`.
pub struct Step { pub to: u32, pub run: fn(&Path) -> Result<(), String> }

/// Migraciones de esta versión (todavía ninguna: el formato 1 es el primero con número).
pub const STEPS: &[Step] = &[];

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Current,
    Migrated { from: u32, to: u32, backup: PathBuf },
    /// La migración falló y se restauró el respaldo (los datos quedaron como estaban).
    Restored { from: u32, error: String },
    /// Datos de una versión más nueva: no se toca nada.
    Newer { found: u32, supported: u32 },
}

fn version_file(root: &Path) -> PathBuf { root.join("version-datos") }

pub fn read_version(root: &Path) -> u32 {
    std::fs::read_to_string(version_file(root)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(1)
}

fn write_version(root: &Path, v: u32) -> std::io::Result<()> {
    let tmp = root.join("version-datos.tmp-939");
    std::fs::write(&tmp, v.to_string())?;
    std::fs::rename(tmp, version_file(root))
}

/// Carpetas que no se respaldan ni se migran (mundos, puntos, copias, caché, Java, registros).
const SKIP_DIRS: &[&str] = &["server", "backups", "restore", ".restore-staging", "mods-apartados", "bloques", "blocks", "snapshots", "cache", "runtimes", "logs", "Papelera", "respaldos-datos", "versions", "libraries"];
const MAX_FILE: u64 = 8 * 1024 * 1024;

fn backup_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        if m.is_dir() {
            if !SKIP_DIRS.iter().any(|d| name.eq_ignore_ascii_case(d)) { backup_files(root, &e.path(), out); }
        } else if m.is_file() && m.len() <= MAX_FILE && !name.ends_with(".tmp-939") {
            if let Ok(rel) = e.path().strip_prefix(root) { out.push(rel.to_path_buf()); }
        }
    }
}

/// Copia los datos (sin mundos) a un respaldo nuevo y devuelve su carpeta.
pub fn backup(root: &Path, from: u32) -> Result<PathBuf, String> {
    let dest = root.join("respaldos-datos").join(format!("v{from}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let mut files = vec![];
    backup_files(root, root, &mut files);
    for rel in &files {
        let to = dest.join(rel);
        if let Some(p) = to.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::fs::copy(root.join(rel), &to).map_err(|e| format!("respaldar {}: {e}", rel.display()))?;
    }
    std::fs::write(dest.join("LEEME.txt"), format!("Respaldo automático de los datos de 939 Host (formato {from}) antes de una migración.\nNo incluye mundos ni copias de seguridad, que no se migran.\n")).map_err(|e| e.to_string())?;
    // Se conservan los 3 respaldos más recientes.
    if let Ok(rd) = std::fs::read_dir(root.join("respaldos-datos")) {
        let mut all: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        all.sort();
        while all.len() > 3 { let _ = std::fs::remove_dir_all(all.remove(0)); }
    }
    Ok(dest)
}

/// Vuelve a poner los archivos del respaldo en su lugar.
pub fn restore(root: &Path, backup: &Path) -> Result<(), String> {
    let mut files = vec![];
    backup_files(backup, backup, &mut files);
    for rel in files {
        if rel == Path::new("LEEME.txt") { continue; }
        let to = root.join(&rel);
        if let Some(p) = to.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::fs::copy(backup.join(&rel), &to).map_err(|e| format!("restaurar {}: {e}", rel.display()))?;
    }
    Ok(())
}

/// Comprobación después de migrar: todo lo que la app necesita para abrir se puede leer.
pub fn validate(root: &Path) -> Result<(), String> {
    if let Ok(rd) = std::fs::read_dir(root.join("Servers")) {
        for e in rd.flatten() {
            let p = e.path().join("instance.json");
            if p.exists() {
                let b = std::fs::read(&p).map_err(|e| e.to_string())?;
                serde_json::from_slice::<crate::model::ServerConfig>(&b).map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
    }
    let s = root.join("settings.json");
    if s.exists() { serde_json::from_slice::<serde_json::Value>(&std::fs::read(&s).map_err(|e| e.to_string())?).map_err(|e| format!("settings.json: {e}"))?; }
    Ok(())
}

/// Lleva los datos de `root` al formato `current` con los pasos dados.
pub fn migrate_with(root: &Path, current: u32, steps: &[Step]) -> Outcome {
    let found = read_version(root);
    if found == current {
        if !version_file(root).exists() { let _ = write_version(root, current); }
        return Outcome::Current;
    }
    if found > current { return Outcome::Newer { found, supported: current }; }
    let backup = match backup(root, found) { Ok(b) => b, Err(e) => return Outcome::Restored { from: found, error: format!("no se pudo respaldar: {e}") } };
    let mut v = found;
    let result = (|| {
        while v < current {
            let step = steps.iter().find(|s| s.to == v + 1).ok_or_else(|| format!("falta la migración al formato {}", v + 1))?;
            (step.run)(root)?;
            v += 1;
        }
        validate(root)?;
        write_version(root, current).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => Outcome::Migrated { from: found, to: current, backup },
        Err(error) => {
            let _ = restore(root, &backup);
            let _ = write_version(root, found);
            Outcome::Restored { from: found, error }
        }
    }
}

/// Lo que hace la app al abrir.
pub fn ensure() -> Outcome { migrate_with(paths::root(), DATA_VERSION, STEPS) }

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("Servers/s1/server/world")).unwrap();
        std::fs::write(r.join("Servers/s1/server/world/level.dat"), b"mundo").unwrap();
        std::fs::write(r.join("Servers/s1/estado.json"), br#"{"a":1}"#).unwrap();
        std::fs::write(r.join("settings.json"), br#"{"tema":"oscuro"}"#).unwrap();
        std::fs::write(r.join("amigos.bin"), b"cifrado").unwrap();
        t
    }

    fn ok_step(r: &Path) -> Result<(), String> { std::fs::write(r.join("settings.json"), br#"{"tema":"oscuro","nuevo":true}"#).map_err(|e| e.to_string()) }
    fn bad_step(r: &Path) -> Result<(), String> {
        std::fs::write(r.join("settings.json"), b"{roto").unwrap();
        std::fs::write(r.join("amigos.bin"), b"pisado").unwrap();
        Err("falla a mitad de camino".into())
    }
    fn breaks_validation(r: &Path) -> Result<(), String> { std::fs::write(r.join("settings.json"), b"no es json").map_err(|e| e.to_string()) }

    #[test]
    fn sin_numero_es_el_formato_1_y_queda_anotado() {
        let t = data();
        assert_eq!(migrate_with(t.path(), 1, &[]), Outcome::Current);
        assert_eq!(read_version(t.path()), 1);
    }

    #[test]
    fn migracion_con_respaldo_sin_mundos() {
        let t = data();
        let r = t.path();
        let Outcome::Migrated { from: 1, to: 2, backup } = migrate_with(r, 2, &[Step { to: 2, run: ok_step }]) else { panic!() };
        assert_eq!(read_version(r), 2);
        assert!(std::fs::read_to_string(r.join("settings.json")).unwrap().contains("nuevo"));
        assert_eq!(std::fs::read(backup.join("amigos.bin")).unwrap(), b"cifrado");
        assert!(backup.join("Servers/s1/estado.json").exists());
        assert!(!backup.join("Servers/s1/server").exists(), "los mundos no se copian ni se migran");
        assert_eq!(std::fs::read(r.join("Servers/s1/server/world/level.dat")).unwrap(), b"mundo");
    }

    #[test]
    fn falla_a_mitad_o_en_la_validacion_y_se_restaura() {
        for step in [bad_step as fn(&Path) -> Result<(), String>, breaks_validation] {
            let t = data();
            let r = t.path();
            let out = migrate_with(r, 2, &[Step { to: 2, run: step }]);
            assert!(matches!(out, Outcome::Restored { from: 1, .. }), "{out:?}");
            assert_eq!(read_version(r), 1);
            assert_eq!(std::fs::read_to_string(r.join("settings.json")).unwrap(), r#"{"tema":"oscuro"}"#);
            assert_eq!(std::fs::read(r.join("amigos.bin")).unwrap(), b"cifrado");
        }
        // Falta un paso intermedio: tampoco se toca nada.
        let t = data();
        assert!(matches!(migrate_with(t.path(), 3, &[Step { to: 3, run: ok_step }]), Outcome::Restored { .. }));
        assert_eq!(read_version(t.path()), 1);
    }

    #[test]
    fn datos_de_una_version_mas_nueva_no_se_tocan() {
        let t = data();
        std::fs::write(t.path().join("version-datos"), "5").unwrap();
        assert_eq!(migrate_with(t.path(), 2, &[Step { to: 2, run: ok_step }]), Outcome::Newer { found: 5, supported: 2 });
        assert_eq!(std::fs::read_to_string(t.path().join("settings.json")).unwrap(), r#"{"tema":"oscuro"}"#);
        assert!(!t.path().join("respaldos-datos").exists());
    }

    #[test]
    fn solo_quedan_tres_respaldos() {
        let t = data();
        for i in 0..5 { std::fs::create_dir_all(t.path().join(format!("respaldos-datos/v1-2020010{i}-000000"))).unwrap(); }
        backup(t.path(), 1).unwrap();
        assert_eq!(std::fs::read_dir(t.path().join("respaldos-datos")).unwrap().count(), 3);
    }
}
