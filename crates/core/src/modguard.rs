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
/// `.jar` nuevo, dentro del candado. `from` es la PC de origen, autenticada por la conexión (o el
/// origen de un punto de restauración, que fija quien lo recibe). Lo recibido sin aprobar pasa a la
/// cuarentena en ese mismo momento: nunca queda en la carpeta que se carga.
/// Devuelve el resultado de `place` y cuántos mods llegaron. Aunque `place` falle a mitad de
/// camino, lo que alcanzó a llegar queda registrado y apartado.
pub fn foreign_write<T>(id: &str, from: &str, place: impl FnOnce() -> AppResult<T>) -> AppResult<(T, usize)> {
    let _g = ingress();
    let before = snapshot(id);
    let r = place();
    let n = record_received(id, &before, from);
    Ok((r?, n))
}

fn mods_dir(id: &str) -> std::path::PathBuf { paths::server_run_dir(id).join("mods") }
fn quarantine_dir(id: &str) -> std::path::PathBuf { paths::server_run_dir(id).join(QUARANTINE) }
fn origins_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("mods-recibidos-info.json") }
fn decisions_path(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("mods-decisiones.json") }

/// Huellas de los `.jar` que llegaron desde otra PC.
pub fn received(id: &str) -> HashSet<String> { paths::read_json(&ingress_path(id)).unwrap_or_default() }

/// De dónde y cuándo llegó cada mod recibido (por huella).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    /// Id de la PC que lo mandó.
    pub from: String,
    pub at: String,
    /// Nombre con el que llegó (solo informativo: se identifica por la huella).
    pub name: String,
}

pub fn origins(id: &str) -> BTreeMap<String, Origin> { paths::read_json(&origins_path(id)).unwrap_or_default() }

/// Foto de los `.jar` actuales (antes de que entre algo de otra PC).
fn snapshot(id: &str) -> HashSet<String> { jars(&mods_dir(id)).into_values().collect() }

/// Después de que entró contenido de otra PC: lo que no estaba antes queda marcado como recibido y,
/// si no está aprobado (o está rechazado), pasa a la cuarentena.
/// Falla cerrado: un mod que no se pudo leer, o un registro que no se pudo guardar, también va a
/// la cuarentena.
fn record_received(id: &str, before: &HashSet<String>, from: &str) -> usize {
    let mut r = received(id);
    let mut info = origins(id);
    let (now, unreadable) = scan(&mods_dir(id));
    let mut fresh = vec![];
    let at = chrono::Utc::now().to_rfc3339();
    for (name, h) in now {
        if before.contains(&h) { continue; }
        let new_hash = r.insert(h.clone());
        info.entry(h.clone()).or_insert_with(|| Origin { from: from.to_string(), at: at.clone(), name: name.clone() });
        fresh.push((name, h, new_hash));
    }
    let n = fresh.iter().filter(|f| f.2).count();
    let saved = fresh.is_empty() || paths::write_json(&ingress_path(id), &r).is_ok();
    if !fresh.is_empty() { let _ = paths::write_json(&origins_path(id), &info); }
    let rules = group_rules(id);
    let mut hold: Vec<String> = unreadable;
    for (name, h, _) in fresh {
        let blocked = match &rules {
            Some(g) => g.rejected.contains(&h) || (g.strict && !g.approved.contains(&h)),
            None => false,
        };
        if blocked || !saved { hold.push(name); }
    }
    for name in hold { let _ = move_to_quarantine(id, &name); }
    n
}

fn move_to_quarantine(id: &str, name: &str) -> AppResult<()> {
    let q = quarantine_dir(id);
    let dst = q.join(name);
    std::fs::create_dir_all(dst.parent().unwrap_or(&q)).map_err(|e| AppError::io("No se pudo crear la cuarentena de mods", e))?;
    if dst.exists() { let _ = std::fs::remove_file(&dst); }
    paths::rename_retry(mods_dir(id).join(name), &dst).map_err(|e| AppError::io("No se pudo apartar un mod sin aprobar", e))
}

/// Reglas del grupo (de la lista firmada por la propietaria). Servidores no compartidos: sin control.
struct Rules { strict: bool, approved: HashSet<String>, rejected: HashSet<String> }

fn group_rules(id: &str) -> Option<Rules> {
    let s = crate::network::load_share(id);
    if !s.enabled { return None; }
    let strict = s.mod_policy.as_deref().unwrap_or("estricta") != "confiar-coanfitrionas";
    Some(Rules { strict, approved: s.approved_mods.iter().cloned().collect(), rejected: s.rejected_mods.iter().cloned().collect() })
}

/// Antes de encender: aparta los `.jar` recibidos sin aprobar y los rechazados, y devuelve los que
/// vuelven de la cuarentena porque ya se aprobaron. Resultado: (apartados, devueltos).
pub fn enforce(id: &str) -> AppResult<(Vec<String>, Vec<String>)> {
    let Some(g) = group_rules(id) else { return Ok((vec![], vec![])) };
    let _g = ingress();
    let q = quarantine_dir(id);
    let mut back = vec![];
    for (name, h) in jars(&q) {
        if g.rejected.contains(&h) { continue; }
        if g.approved.contains(&h) || !g.strict {
            let dst = mods_dir(id).join(&name);
            if let Some(parent) = dst.parent() { let _ = std::fs::create_dir_all(parent); }
            if !dst.exists() && std::fs::rename(q.join(&name), &dst).is_ok() { back.push(name); }
        }
    }
    let rec = received(id);
    let mut moved = vec![];
    let (now, unreadable) = scan(&mods_dir(id));
    for (name, h) in now {
        // Un rechazo vale siempre, aunque el archivo se haya instalado en esta PC.
        if g.rejected.contains(&h) || (g.strict && rec.contains(&h) && !g.approved.contains(&h)) {
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

/// Un mod en la cuarentena de esta PC.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quarantined {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    /// PC de origen (id) y cuándo llegó, si se registró.
    pub from: Option<String>,
    pub at: Option<String>,
    pub rejected: bool,
}

/// Mods apartados en esta PC: los que esperan aprobación y los rechazados que siguen ahí.
pub fn quarantined(id: &str) -> Vec<Quarantined> {
    let q = quarantine_dir(id);
    let info = origins(id);
    let rejected: HashSet<String> = crate::network::load_share(id).rejected_mods.into_iter().collect();
    jars(&q).into_iter().map(|(name, h)| Quarantined {
        size: std::fs::metadata(q.join(&name)).map(|m| m.len()).unwrap_or(0),
        from: info.get(&h).map(|o| o.from.clone()), at: info.get(&h).map(|o| o.at.clone()),
        rejected: rejected.contains(&h), name, sha256: h,
    }).collect()
}

/// Mods apartados esperando aprobación: (nombre, huella). No incluye los rechazados.
pub fn pending(id: &str) -> Vec<(String, String)> {
    quarantined(id).into_iter().filter(|q| !q.rejected).map(|q| (q.name, q.sha256)).collect()
}

/// Ruta del archivo en cuarentena con esa huella (para analizarlo). Nunca de la carpeta activa.
pub fn quarantined_path(id: &str, sha256: &str) -> Option<std::path::PathBuf> {
    let q = quarantine_dir(id);
    jars(&q).into_iter().find(|(_, h)| h == sha256).map(|(n, _)| q.join(n))
}

/// Borra de la cuarentena los archivos con esa huella (rechazados que no se quieren conservar).
pub fn discard(id: &str, sha256: &str) -> usize {
    let _g = ingress();
    let q = quarantine_dir(id);
    let mut n = 0;
    for (name, h) in jars(&q) {
        if h == sha256 && std::fs::remove_file(q.join(&name)).is_ok() { n += 1; }
    }
    n
}

/// `.jar` instalados en esta PC por su usuario (no recibidos ni rechazados): la propietaria los
/// aprueba solos. Si en ese momento está entrando contenido de otra PC, devuelve una lista vacía:
/// se aprueba en la vuelta siguiente, ya con lo recibido registrado (no esperar evita trabar el
/// ciclo del grupo durante una restauración larga, y no aprobar nunca habilita nada).
pub fn local_jars(id: &str) -> Vec<String> {
    let _g = match INGRESS.try_lock() {
        Ok(g) => g,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return vec![],
    };
    let rec = received(id);
    let rejected: HashSet<String> = crate::network::load_share(id).rejected_mods.into_iter().collect();
    jars(&mods_dir(id)).into_values().filter(|h| !rec.contains(h) && !rejected.contains(h)).collect()
}

/// Una decisión de la propietaria sobre un mod.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub at: String,
    pub sha256: String,
    pub name: String,
    /// "aprobado" | "rechazado" | "descartado" | "propio"
    pub decision: String,
    /// Estado del análisis en el momento de decidir (si había).
    #[serde(default)]
    pub analysis: Option<String>,
}

pub fn decisions(id: &str) -> Vec<Decision> { paths::read_json(&decisions_path(id)).unwrap_or_default() }

/// Registra una decisión (se guardan las últimas 500).
pub fn log_decision(id: &str, sha256: &str, decision: &str) {
    let name = origins(id).get(sha256).map(|o| o.name.clone())
        .or_else(|| quarantined(id).into_iter().find(|q| q.sha256 == sha256).map(|q| q.name))
        .unwrap_or_default();
    let analysis = crate::modscan::cached(sha256).map(|r| r.status);
    let mut v = decisions(id);
    v.push(Decision { at: chrono::Utc::now().to_rfc3339(), sha256: sha256.into(), name, decision: decision.into(), analysis });
    if v.len() > 500 { let drop = v.len() - 500; v.drain(..drop); }
    let _ = paths::write_json(&decisions_path(id), &v);
}

#[cfg(test)]
mod t {
    use super::*;

    fn group(tag: &str) -> String {
        let _ = paths::init(std::env::temp_dir().join(format!("h939-modguard-{}", std::process::id())));
        let id = format!("mg-{tag}-{}", std::process::id());
        std::fs::create_dir_all(mods_dir(&id)).unwrap();
        crate::network::save_share(&id, &crate::network::ShareConfig { enabled: true, ..Default::default() }).unwrap();
        id
    }
    fn set_share(id: &str, f: impl FnOnce(&mut crate::network::ShareConfig)) {
        let mut s = crate::network::load_share(id);
        f(&mut s);
        crate::network::save_share(id, &s).unwrap();
    }
    fn arrive(id: &str, files: &[(&str, &[u8])]) -> usize {
        let m = mods_dir(id);
        foreign_write(id, "pc-coanfitriona", || {
            for (n, b) in files { let p = m.join(n); std::fs::create_dir_all(p.parent().unwrap()).unwrap(); std::fs::write(p, b).unwrap(); }
            Ok(())
        }).unwrap().1
    }
    fn sha(b: &[u8]) -> String { hex::encode(sha2::Sha256::digest(b)) }

    #[test]
    fn lo_recibido_va_directo_a_la_cuarentena() {
        let id = &group("directo");
        let m = mods_dir(id);
        std::fs::write(m.join("propio.jar"), b"instalado aca").unwrap();
        assert_eq!(arrive(id, &[("nuevo.jar", b"codigo de otra pc")]), 1);
        assert!(!m.join("nuevo.jar").exists(), "lo recibido sin aprobar nunca queda en la carpeta activa");
        assert!(m.join("propio.jar").exists(), "lo instalado en esta PC no se toca");
        assert_eq!(pending(id), vec![("nuevo.jar".to_string(), sha(b"codigo de otra pc"))]);
        let (moved, back) = enforce(id).unwrap();
        assert!(moved.is_empty() && back.is_empty());
        // La propietaria aprueba la huella exacta: vuelve al próximo encendido.
        set_share(id, |s| s.approved_mods = vec![sha(b"codigo de otra pc")]);
        assert_eq!(enforce(id).unwrap().1, vec!["nuevo.jar".to_string()]);
        // Un byte distinto (mismo nombre) es otro archivo: la aprobación no vale.
        assert_eq!(arrive(id, &[("nuevo.jar", b"codigo de otra pcX")]), 1);
        assert!(quarantined(id).iter().any(|q| q.sha256 == sha(b"codigo de otra pcX")));
        assert!(!jars(&m).values().any(|h| *h == sha(b"codigo de otra pcX")), "el reemplazo no queda activo");
        // Escondidos en subcarpetas (Forge viejo) o como .zip: también.
        assert_eq!(arrive(id, &[("1.12.2/escondido.jar", b"otra pc"), ("viejo.ZIP", b"otra pc zip")]), 2);
        assert!(!m.join("1.12.2/escondido.jar").exists() && !m.join("viejo.ZIP").exists());
        // Una subida a medias no es un mod: los .tmp no se cargan ni se cuentan.
        std::fs::write(m.join(".939-subida-x.jar.tmp"), b"a medias").unwrap();
        assert!(!jars(&m).contains_key(".939-subida-x.jar.tmp"));
        // Política de confianza total: vuelve todo lo no rechazado.
        set_share(id, |s| s.mod_policy = Some("confiar-coanfitrionas".into()));
        let (moved, back) = enforce(id).unwrap();
        assert!(moved.is_empty() && back.contains(&"1.12.2/escondido.jar".to_string()), "{back:?}");
    }

    #[test]
    fn un_rechazo_vale_siempre() {
        let id = &group("rechazo");
        let m = mods_dir(id);
        let bad = b"mod rechazado";
        set_share(id, |s| { s.rejected_mods = vec![sha(bad)]; s.mod_policy = Some("confiar-coanfitrionas".into()); });
        // Ni con confianza total en las coanfitrionas.
        arrive(id, &[("r.jar", bad)]);
        assert!(!m.join("r.jar").exists());
        assert!(quarantined(id).iter().any(|q| q.rejected), "queda apartado y marcado como rechazado");
        assert!(pending(id).is_empty(), "un rechazado no aparece como pendiente");
        assert!(enforce(id).unwrap().1.is_empty(), "no vuelve de la cuarentena");
        // Ni aunque alguien lo copie a mano en la carpeta activa de esta PC.
        std::fs::write(m.join("a-mano.jar"), bad).unwrap();
        assert!(!local_jars(id).contains(&sha(bad)), "no se aprueba solo como propio");
        assert_eq!(enforce(id).unwrap().0, vec!["a-mano.jar".to_string()]);
        // Ni aunque figure también como aprobado (el rechazo gana).
        set_share(id, |s| s.approved_mods = vec![sha(bad)]);
        assert!(enforce(id).unwrap().1.is_empty());
        // Descartar lo borra de la cuarentena.
        assert_eq!(discard(id, &sha(bad)), 2);
        assert!(quarantined(id).is_empty());
    }

    #[test]
    fn procedencia_y_registro_de_decisiones() {
        let id = &group("registro-dec");
        arrive(id, &[("x.jar", b"de la coanfitriona")]);
        let q = quarantined(id);
        assert_eq!(q[0].from.as_deref(), Some("pc-coanfitriona"));
        assert!(q[0].at.is_some() && q[0].size == 18);
        log_decision(id, &sha(b"de la coanfitriona"), "rechazado");
        let d = decisions(id);
        assert_eq!((d[0].name.as_str(), d[0].decision.as_str()), ("x.jar", "rechazado"));
        for _ in 0..600 { log_decision(id, &sha(b"y"), "aprobado"); }
        assert_eq!(decisions(id).len(), 500, "el registro tiene un tope");
    }

    /// Red team, vuelta 7 (lo encontró la CI de Windows): mientras entra contenido de otra PC, el
    /// `.jar` ya estaba en `mods/` pero todavía no figuraba como recibido. Si en ese instante corría
    /// la aprobación automática de la propietaria, lo tomaba como instalado por ella y lo aprobaba
    /// (firmado y para siempre). Acá la aprobación corre justo en esa ventana.
    #[test]
    fn aprobacion_automatica_no_ve_lo_que_esta_entrando() {
        let id = group("carrera");
        let m = mods_dir(&id);
        let troyano = sha(b"codigo de otra pc");
        let seen_during = foreign_write(&id, "pc-coanfitriona", || {
            std::fs::write(m.join("troyano.jar"), b"codigo de otra pc").unwrap();
            let id2 = id.clone();
            Ok(std::thread::spawn(move || local_jars(&id2)).join().unwrap())
        }).unwrap().0;
        assert!(!seen_during.contains(&troyano), "la aprobación automática vio el mod mientras entraba");
        assert!(!local_jars(&id).contains(&troyano), "después de entrar figura como recibido, no como propio");
        assert!(!m.join("troyano.jar").exists() && pending(&id).len() == 1);
    }

    /// Si el registro de recibidos no se puede guardar, lo que llegó no queda en `mods/`.
    #[test]
    fn registro_que_no_se_guarda_falla_cerrado() {
        let id = group("registro");
        let m = mods_dir(&id);
        // Con confianza total, lo recibido iría directo a la carpeta activa; sin registro, no.
        set_share(&id, |s| s.mod_policy = Some("confiar-coanfitrionas".into()));
        std::fs::create_dir_all(ingress_path(&id)).unwrap(); // una carpeta donde va el archivo: no se puede escribir
        assert_eq!(arrive(&id, &[("llegado.jar", b"de otra pc")]), 1);
        assert!(!m.join("llegado.jar").exists(), "sin registro, el mod no puede quedar donde se carga");
        assert_eq!(pending(&id).len(), 1, "queda en la cuarentena");
        assert!(local_jars(&id).is_empty());
    }
}
