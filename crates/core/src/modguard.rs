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
/// con `/` → huella. No sigue enlaces. Los que no se pudieron leer no figuran: para decisiones de
/// seguridad usar [`scan`], que los informa.
pub fn jars(dir: &Path) -> BTreeMap<String, String> { scan(dir).0 }

/// Como [`jars`], más la lista de mods cuya huella no se pudo calcular (en Windows, un archivo
/// recién copiado puede estar bloqueado un instante, por ejemplo por el antivirus). Cada uno se
/// reintenta unas veces antes de darlo por ilegible.
pub fn scan(dir: &Path) -> (BTreeMap<String, String>, Vec<String>) {
    fn walk(base: &Path, dir: &Path, depth: u32, out: &mut BTreeMap<String, String>, bad: &mut Vec<String>) {
        if depth > 6 { return; }
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() { walk(base, &p, depth + 1, out, bad); continue; }
            if !t.is_file() || !is_mod_file(&e.file_name().to_string_lossy()) { continue; }
            let Ok(rel) = p.strip_prefix(base) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let mut h = sha256_file(&p);
            for wait in [50u64, 150, 400] {
                if h.is_some() { break; }
                std::thread::sleep(std::time::Duration::from_millis(wait));
                h = sha256_file(&p);
            }
            match h { Some(h) => { out.insert(rel, h); } None => bad.push(rel) }
        }
    }
    let (mut out, mut bad) = (BTreeMap::new(), vec![]);
    walk(dir, dir, 0, &mut out, &mut bad);
    (out, bad)
}

/// Candado de las entradas desde otra PC. Lo que llega (punto restaurado, subida o copia por el
/// administrador remoto) se coloca y se registra como recibido **sin que nadie mire en el medio**:
/// la aprobación automática de la propietaria ([`local_jars`]) y el control antes de encender
/// ([`enforce`]) toman el mismo candado. Sin él, un `.jar` recién llegado y todavía sin registrar
/// parecía instalado por la propietaria y se aprobaba solo (red team, vuelta 7).
static INGRESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn ingress() -> std::sync::MutexGuard<'static, ()> { INGRESS.lock().unwrap_or_else(|e| e.into_inner()) }

/// Ejecuta `place` (lo que pone en `mods/` contenido de otra PC) y registra como recibido todo
/// `.jar` nuevo, dentro del candado. Devuelve el resultado de `place` y cuántos mods llegaron.
/// Aunque `place` falle a mitad de camino, lo que alcanzó a llegar queda registrado.
pub fn foreign_write<T>(id: &str, place: impl FnOnce() -> AppResult<T>) -> AppResult<(T, usize)> {
    let _g = ingress();
    let before = snapshot(id);
    let r = place();
    let n = record_received(id, &before);
    Ok((r?, n))
}

fn mods_dir(id: &str) -> std::path::PathBuf { paths::server_run_dir(id).join("mods") }

/// Huellas de los `.jar` que llegaron desde otra PC.
pub fn received(id: &str) -> HashSet<String> { paths::read_json(&ingress_path(id)).unwrap_or_default() }

/// Foto de los `.jar` actuales (antes de que entre algo de otra PC).
fn snapshot(id: &str) -> HashSet<String> { jars(&mods_dir(id)).into_values().collect() }

/// Después de que entró contenido de otra PC: lo que no estaba antes queda marcado como recibido.
/// Falla cerrado: un mod que no se pudo leer, o un registro que no se pudo guardar, deja esos
/// mods en la cuarentena en lugar de en `mods/`.
fn record_received(id: &str, before: &HashSet<String>) -> usize {
    let mut r = received(id);
    let (now, unreadable) = scan(&mods_dir(id));
    let mut fresh = vec![];
    for (name, h) in now {
        if !before.contains(&h) && r.insert(h) { fresh.push(name); }
    }
    let n = fresh.len();
    let saved = n == 0 || paths::write_json(&ingress_path(id), &r).is_ok();
    let mut hold: Vec<String> = unreadable;
    if !saved { hold.extend(fresh); }
    for name in hold { let _ = move_to_quarantine(id, &name); }
    n
}

fn move_to_quarantine(id: &str, name: &str) -> AppResult<()> {
    let q = paths::server_run_dir(id).join(QUARANTINE);
    let dst = q.join(name);
    std::fs::create_dir_all(dst.parent().unwrap_or(&q)).map_err(|e| AppError::io("No se pudo crear la cuarentena de mods", e))?;
    if dst.exists() { let _ = std::fs::remove_file(&dst); }
    paths::rename_retry(mods_dir(id).join(name), &dst).map_err(|e| AppError::io("No se pudo apartar un mod sin aprobar", e))
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
    let _g = ingress();
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
    let (now, unreadable) = scan(&mods_dir(id));
    for (name, h) in now {
        if rec.contains(&h) && !approved.contains(&h) {
            move_to_quarantine(id, &name)?;
            moved.push(name);
        }
    }
    // Sin huella no se puede saber si está aprobado: no se carga. Si ni siquiera se puede apartar,
    // no se enciende.
    for name in unreadable {
        move_to_quarantine(id, &name).map_err(|_| AppError::new("mods", format!("No se pudo leer «{name}» para comprobar si está aprobado; el servidor no se encendió.")))?;
        moved.push(name);
    }
    Ok((moved, back))
}

/// Mods apartados esperando aprobación: (nombre, huella).
pub fn pending(id: &str) -> Vec<(String, String)> {
    jars(&paths::server_run_dir(id).join(QUARANTINE)).into_iter().collect()
}

/// `.jar` instalados en esta PC por su usuario (no recibidos): la propietaria los aprueba solos.
/// Si en ese momento está entrando contenido de otra PC, devuelve una lista vacía: se aprueba en
/// la vuelta siguiente, ya con lo recibido registrado (no esperar evita trabar el ciclo del grupo
/// durante una restauración larga, y no aprobar nunca habilita nada).
pub fn local_jars(id: &str) -> Vec<String> {
    let _g = match INGRESS.try_lock() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return vec![],
    };
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

    fn group(tag: &str) -> String {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-modguard-{}", std::process::id())));
        let id = format!("mg-{tag}-{}", std::process::id());
        std::fs::create_dir_all(mods_dir(&id)).unwrap();
        crate::network::save_share(&id, &crate::network::ShareConfig { enabled: true, ..Default::default() }).unwrap();
        id
    }

    /// Red team, vuelta 7 (lo encontró la CI de Windows): mientras entra contenido de otra PC, el
    /// `.jar` ya estaba en `mods/` pero todavía no figuraba como recibido. Si en ese instante corría
    /// la aprobación automática de la propietaria, lo tomaba como instalado por ella y lo aprobaba
    /// (firmado y para siempre). Acá la aprobación corre justo en esa ventana.
    #[test]
    fn aprobacion_automatica_no_ve_lo_que_esta_entrando() {
        let id = group("carrera");
        let m = mods_dir(&id);
        let troyano = sha256_file_of(b"codigo de otra pc");
        let seen_during = foreign_write(&id, || {
            std::fs::write(m.join("troyano.jar"), b"codigo de otra pc").unwrap();
            let id2 = id.clone();
            Ok(std::thread::spawn(move || local_jars(&id2)).join().unwrap())
        }).unwrap().0;
        assert!(!seen_during.contains(&troyano), "la aprobación automática vio el mod mientras entraba");
        assert!(!local_jars(&id).contains(&troyano), "después de entrar figura como recibido, no como propio");
        let (moved, _) = enforce(&id).unwrap();
        assert_eq!(moved, vec!["troyano.jar".to_string()]);
    }

    /// Si el registro de recibidos no se puede guardar, lo que llegó no queda en `mods/`.
    #[test]
    fn registro_que_no_se_guarda_falla_cerrado() {
        let id = group("registro");
        let m = mods_dir(&id);
        std::fs::create_dir_all(ingress_path(&id)).unwrap(); // una carpeta donde va el archivo: no se puede escribir
        let (_, n) = foreign_write(&id, || { std::fs::write(m.join("llegado.jar"), b"de otra pc").unwrap(); Ok(()) }).unwrap();
        assert_eq!(n, 1);
        assert!(!m.join("llegado.jar").exists(), "sin registro, el mod no puede quedar donde se carga");
        assert_eq!(pending(&id).len(), 1, "queda en la cuarentena esperando aprobación");
        assert!(local_jars(&id).is_empty());
        let (moved, back) = enforce(&id).unwrap();
        assert!(moved.is_empty() && back.is_empty(), "sigue apartado: {moved:?} {back:?}");
    }

    fn sha256_file_of(b: &[u8]) -> String { hex::encode(sha2::Sha256::digest(b)) }
}
