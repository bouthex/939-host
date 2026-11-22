//! Ubicaciones en disco. Todo vive bajo %LOCALAPPDATA%\939Host salvo que se configure otra cosa.
//! Nunca se escribe dentro de la instalación de Minecraft del usuario.

use crate::error::{AppError, AppResult};
use once_cell::sync::OnceCell;
use std::path::{Path, PathBuf};

static ROOT: OnceCell<PathBuf> = OnceCell::new();

pub fn init(root: PathBuf) -> AppResult<()> {
    for sub in ["Servers", "runtimes", "cache", "cache/jars", "identity", "logs", "Papelera"] {
        std::fs::create_dir_all(root.join(sub)).map_err(|e| AppError::io("No se pudo crear la carpeta de datos de 939 Host", e))?;
    }
    let _ = ROOT.set(root);
    Ok(())
}

pub fn root() -> &'static Path {
    ROOT.get().expect("paths::init no fue llamado")
}
pub fn servers() -> PathBuf { root().join("Servers") }
pub fn runtimes() -> PathBuf { root().join("runtimes") }
pub fn cache() -> PathBuf { root().join("cache") }
pub fn identity() -> PathBuf { root().join("identity") }
pub fn trash() -> PathBuf { root().join("Papelera") }

/// Ruta única dentro de la Papelera (fecha con milisegundos + sufijo aleatorio): dos
/// operaciones en el mismo segundo nunca chocan.
pub fn trash_slot(label: &str) -> PathBuf {
    let mut b = [0u8; 2];
    let _ = getrandom::getrandom(&mut b);
    trash().join(format!("{label}-{}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"), hex::encode(b)))
}

/// Mueve un archivo o carpeta, reintentando unos segundos si Windows lo tiene bloqueado.
///
/// En Windows, el antivirus o el indexador abren por un momento los archivos recién escritos
/// (después de un apagado, una restauración o una copia) y renombrar la carpeta falla con
/// «Acceso denegado» aunque nadie la esté usando de verdad. Se reintenta hasta ~8 s.
pub fn rename_retry(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    let mut wait = std::time::Duration::from_millis(150);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // 5 = acceso denegado, 32 = archivo en uso por otro proceso.
            Err(e) if std::time::Instant::now() < deadline
                && (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5) | Some(32))) => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(std::time::Duration::from_secs(1));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Carpeta de un servidor: contiene `instance.json`, `server/` (lo que ve Java) y `backups/`.
/// Id de servidor válido: minúsculas ASCII, dígitos y guiones, sin empezar con guion, hasta 48.
/// Los ids llegan también desde la red (otras PCs): nunca pueden salir de la carpeta de servidores.
pub fn valid_server_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && !id.starts_with('-') && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Carpeta de un servidor. Un id inválido (por ejemplo `../../x` recibido por la red) apunta a
/// una carpeta que nunca existe dentro de `servers`, así que no lee ni escribe fuera de ella.
pub fn server_dir(id: &str) -> PathBuf {
    if valid_server_id(id) { servers().join(id) } else { servers().join("_id-invalido_").join("_") }
}
pub fn server_run_dir(id: &str) -> PathBuf { server_dir(id).join("server") }
pub fn server_backups_dir(id: &str) -> PathBuf { server_dir(id).join("backups") }

/// Escritura atómica: se escribe a un archivo temporal y se renombra. Si se corta la luz
/// a mitad de camino, queda el archivo anterior intacto, nunca uno a medias.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path.parent().ok_or_else(|| AppError::invalid("Ruta inválida"))?;
    std::fs::create_dir_all(dir).map_err(|e| AppError::io("No se pudo crear la carpeta", e))?;
    // Temporal propio de cada escritura: dos hilos que guardan el mismo archivo a la vez (por
    // ejemplo, la lista del grupo desde la red y desde un comando) no se pisan el temporal; antes,
    // el segundo podía encontrarlo ya renombrado («el archivo no existe») y perder su escritura.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!("{name}.{}-{n}.tmp-939", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| AppError::io("No se pudo escribir el archivo", e))?;
    // En Windows, reemplazar un archivo que otro hilo está leyendo en ese instante falla con
    // «acceso denegado»: se reintenta en lugar de perder la escritura.
    rename_retry(&tmp, path).map_err(|e| { let _ = std::fs::remove_file(&tmp); AppError::io("No se pudo reemplazar el archivo", e) })?;
    Ok(())
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> AppResult<T> {
    let bytes = std::fs::read(path).map_err(|e| AppError::io("No se pudo leer la configuración", e))?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_atomic(path, &bytes)
}

/// Copia recursiva de carpetas (solo lectura sobre el origen).
pub fn copy_dir(src: &Path, dst: &Path) -> AppResult<u64> {
    let mut total = 0u64;
    std::fs::create_dir_all(dst).map_err(|e| AppError::io("No se pudo crear la carpeta de destino", e))?;
    for entry in std::fs::read_dir(src).map_err(|e| AppError::io("No se pudo leer la carpeta de origen", e))? {
        let entry = entry.map_err(|e| AppError::io("No se pudo leer la carpeta de origen", e))?;
        let ft = entry.file_type().map_err(|e| AppError::io("No se pudo leer un archivo", e))?;
        let name = entry.file_name();
        // session.lock es un archivo de bloqueo de Minecraft: no se copia.
        if name == "session.lock" { continue; }
        let to = dst.join(&name);
        if ft.is_dir() {
            total += copy_dir(&entry.path(), &to)?;
        } else if ft.is_file() {
            total += std::fs::copy(entry.path(), &to).map_err(|e| AppError::io("No se pudo copiar un archivo del mundo", e))?;
        }
    }
    Ok(total)
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => total += dir_size(&e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

/// Programa de Windows por ruta absoluta (`System32`, o la carpeta de Windows para `explorer.exe`).
/// Nunca se busca por nombre: Windows mira primero la carpeta de la app.
pub fn windows_program(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    if name.eq_ignore_ascii_case("explorer.exe") { root.join(name) } else { root.join("System32").join(name) }
}

#[cfg(test)]
mod escritura_atomica {
    #[test]
    fn escrituras_simultaneas_del_mismo_archivo() {
        let d = std::env::temp_dir().join(format!("939-atomica-{}", std::process::id()));
        let p = d.join("share.json");
        let hs: Vec<_> = (0..8).map(|i| { let p = p.clone(); std::thread::spawn(move || { for j in 0..40 { super::write_atomic(&p, format!("{i}-{j}").as_bytes()).expect("ninguna escritura se pierde"); } }) }).collect();
        for h in hs { h.join().unwrap(); }
        assert!(std::fs::read_dir(&d).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp-939")), "sin temporales");
        let _ = std::fs::remove_dir_all(&d);
    }
}
