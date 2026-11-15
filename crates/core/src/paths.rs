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
pub fn server_dir(id: &str) -> PathBuf { servers().join(id) }
pub fn server_run_dir(id: &str) -> PathBuf { server_dir(id).join("server") }
pub fn server_backups_dir(id: &str) -> PathBuf { server_dir(id).join("backups") }

/// Escritura atómica: se escribe a un archivo temporal y se renombra. Si se corta la luz
/// a mitad de camino, queda el archivo anterior intacto, nunca uno a medias.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path.parent().ok_or_else(|| AppError::invalid("Ruta inválida"))?;
    std::fs::create_dir_all(dir).map_err(|e| AppError::io("No se pudo crear la carpeta", e))?;
    let tmp = path.with_extension("tmp-939");
    std::fs::write(&tmp, bytes).map_err(|e| AppError::io("No se pudo escribir el archivo", e))?;
    std::fs::rename(&tmp, path).map_err(|e| AppError::io("No se pudo reemplazar el archivo", e))?;
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
