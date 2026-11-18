//! Mods recibidos de otras PCs del grupo: no se ejecutan sin la aprobación de la propietaria.
//!
//! Un `.jar` en `mods/` es código que corre con los permisos del usuario de Windows de la PC que
//! aloja el servidor. 939 Host no puede aislarlo (Minecraft no tiene sandbox y un mod puede hacer
//! lo mismo que cualquier programa). Lo que sí hace:
//!
//! * Registra qué `.jar` llegaron **desde otra PC** (por sincronización de un punto creado en otra
//!   PC, o subidos por el administrador remoto de archivos). Lo instalado en esta misma PC por su
//!   usuario no cuenta como recibido.
//! * Antes de encender, todo `.jar` recibido cuya huella SHA-256 no esté en la lista firmada por la
//!   propietaria se aparta a `mods-cuarentena/` y no se carga. Cuando la propietaria lo aprueba,
//!   vuelve solo al próximo encendido.
//! * Un mod aprobado que alguien reemplaza por otro archivo con el mismo nombre tiene otra huella:
//!   vuelve a quedar en cuarentena.
//!
//! Política del grupo (la elige la propietaria, va firmada): `estricta` (por defecto) o
//! `confiar-coanfitrionas` (no se aparta nada; para grupos de confianza total).

use crate::error::{AppError, AppResult};
use crate::paths;
use sha2::Digest;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

const QUARANTINE: &str = "mods-cuarentena";

fn ingress_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("mods-recibidos.json") }

/// Huella SHA-256 de un archivo.
pub fn sha256_file(p: &Path) -> Option<String> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = sha2::Sha256::new();
    std::io::copy(&mut f, &mut h).ok()?;
    Some(hex::encode(h.finalize()))
}

/// ¿Es un archivo que un loader puede cargar como mod? `.jar` (Fabric, Forge, NeoForge) y `.zip`
/// (Forge viejo también los carga).
fn is_mod_file(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".jar") || n.ends_with(".zip")
}

/// Mods en una carpeta y sus subcarpetas (Forge viejo carga `mods/<versión>/`): ruta relativa
/// con `/` → huella. No sigue enlaces.
pub fn jars(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, depth: u32, out: &mut BTreeMap<String, String>) {
        if depth > 6 { return; }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() { walk(base, &p, depth + 1, out); continue; }
            if !t.is_file() || !is_mod_file(&e.file_name().to_string_lossy()) { continue; }
            let Ok(rel) = p.strip_prefix(base) else { continue };
            if let Some(h) = sha256_file(&p) { out.insert(rel.to_string_lossy().replace('\\', "/"), h); }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, 0, &mut out);
    out
}

fn mods_dir(id: &str) -> std::path::PathBuf { paths::server_run_dir(id).join("mods") }

/// Huellas de los `.jar` que llegaron desde otra PC.
pub fn received(id: &str) -> HashSet<String> { paths::read_json(&ingress_path(id)).unwrap_or_default() }

/// Foto de los `.jar` actuales (antes de que entre algo de otra PC).
pub fn snapshot(id: &str) -> HashSet<String> { jars(&mods_dir(id)).into_values().collect() }

/// Después de que entró contenido de otra PC: lo que no estaba antes queda marcado como recibido.
pub fn record_received(id: &str, before: &HashSet<String>) -> usize {
    let mut r = received(id);
    let mut n = 0;
    for h in jars(&mods_dir(id)).into_values() {
        if !before.contains(&h) && r.insert(h) { n += 1; }
    }
    if n > 0 { let _ = paths::write_json(&ingress_path(id), &r); }
    n
}

/// Política y lista aprobada del grupo (de la lista firmada). Servidores no compartidos: sin control.
fn group_rules(id: &str) -> Option<(bool, HashSet<String>)> {
    let s = crate::network::load_share(id);
    if !s.enabled { return None; }
    let strict = s.mod_policy.as_deref().unwrap_or("estricta") != "confiar-coanfitrionas";
    Some((strict, s.approved_mods.iter().cloned().collect()))
}

/// Antes de encender: aparta los `.jar` recibidos sin aprobar y devuelve los que vuelven de la
/// cuarentena porque ya se aprobaron. Resultado: (apartados, devueltos).
pub fn enforce(id: &str) -> AppResult<(Vec<String>, Vec<String>)> {
    let Some((strict, approved)) = group_rules(id) else { return Ok((vec![], vec![])) };
    let run = paths::server_run_dir(id);
    let q = run.join(QUARANTINE);
    let mut back = vec![];
    for (name, h) in jars(&q) {
        if approved.contains(&h) || !strict {
            let dst = mods_dir(id).join(&name);
            if let Some(parent) = dst.parent() { let _ = std::fs::create_dir_all(parent); }
            if !dst.exists() && std::fs::rename(q.join(&name), &dst).is_ok() { back.push(name); }
        }
    }
    if !strict { return Ok((vec![], back)); }
    let rec = received(id);
    let mut moved = vec![];
    for (name, h) in jars(&mods_dir(id)) {
        if rec.contains(&h) && !approved.contains(&h) {
            let dst = q.join(&name);
            std::fs::create_dir_all(dst.parent().unwrap_or(&q)).map_err(|e| AppError::io("No se pudo crear la cuarentena de mods", e))?;
            if dst.exists() { let _ = std::fs::remove_file(&dst); }
            paths::rename_retry(&mods_dir(id).join(&name), &dst).map_err(|e| AppError::io("No se pudo apartar un mod sin aprobar", e))?;
            moved.push(name);
        }
    }
    Ok((moved, back))
}

/// Mods apartados esperando aprobación: (nombre, huella).
pub fn pending(id: &str) -> Vec<(String, String)> {
    jars(&paths::server_run_dir(id).join(QUARANTINE)).into_iter().collect()
}

/// `.jar` instalados en esta PC por su usuario (no recibidos): la propietaria los aprueba solos.
pub fn local_jars(id: &str) -> Vec<String> {
    let rec = received(id);
    jars(&mods_dir(id)).into_values().filter(|h| !rec.contains(h)).collect()
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn received_jars_wait_for_approval() {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-modguard-{}", std::process::id())));
        let id = &format!("mg-{}", std::process::id());
        let m = mods_dir(id);
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("propio.jar"), b"instalado aca").unwrap();
        let mut s = crate::network::ShareConfig { enabled: true, ..Default::default() };
        crate::network::save_share(id, &s).unwrap();
        // Llega contenido de otra PC: un mod nuevo y un reemplazo con el mismo nombre que uno aprobado.
        let before = snapshot(id);
        std::fs::write(m.join("nuevo.jar"), b"codigo de otra pc").unwrap();
        assert_eq!(record_received(id, &before), 1);
        let (moved, _) = enforce(id).unwrap();
        assert_eq!(moved, vec!["nuevo.jar".to_string()], "el recibido sin aprobar no se carga");
        assert!(m.join("propio.jar").exists(), "lo instalado en esta PC no se toca");
        assert_eq!(pending(id).len(), 1);
        // La propietaria lo aprueba: vuelve al próximo encendido.
        s.approved_mods = vec![pending(id)[0].1.clone()];
        crate::network::save_share(id, &s).unwrap();
        let (_, back) = enforce(id).unwrap();
        assert_eq!(back, vec!["nuevo.jar".to_string()]);
        // Reemplazo malicioso de un mod aprobado (mismo nombre, otro contenido): cuarentena otra vez.
        let before = snapshot(id);
        std::fs::write(m.join("nuevo.jar"), b"version alterada").unwrap();
        record_received(id, &before);
        let (moved, _) = enforce(id).unwrap();
        assert_eq!(moved, vec!["nuevo.jar".to_string()]);
        // Un mod recibido escondido en una subcarpeta (Forge viejo carga `mods/<versión>/`) o como
        // `.zip` también queda en cuarentena.
        let before = snapshot(id);
        std::fs::create_dir_all(m.join("1.12.2")).unwrap();
        std::fs::write(m.join("1.12.2/escondido.jar"), b"otra pc").unwrap();
        std::fs::write(m.join("viejo.ZIP"), b"otra pc zip").unwrap();
        assert_eq!(record_received(id, &before), 2);
        let (moved, _) = enforce(id).unwrap();
        assert!(moved.contains(&"1.12.2/escondido.jar".to_string()) && moved.contains(&"viejo.ZIP".to_string()), "{moved:?}");
        assert!(!m.join("1.12.2/escondido.jar").exists() && !m.join("viejo.ZIP").exists());
        // Política de confianza: no se aparta nada.
        s.mod_policy = Some("confiar-coanfitrionas".into());
        crate::network::save_share(id, &s).unwrap();
        let (moved, back) = enforce(id).unwrap();
        assert!(moved.is_empty() && back.contains(&"nuevo.jar".to_string()) && back.contains(&"1.12.2/escondido.jar".to_string()), "{back:?}");
        assert!(m.join("1.12.2/escondido.jar").exists());
    }
}
