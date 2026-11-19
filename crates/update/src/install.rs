//! Cuándo y cómo se instala una actualización ya descargada y verificada.
//!
//! Nunca con un servidor encendido, guardando, sincronizando o pasando el mundo a otra PC: la app
//! informa qué está ocupado y la instalación espera. Siempre con el permiso de la persona.

use crate::download::{file_sha256, DownloadError};
use crate::metadata::Artifact;
use std::path::Path;

/// Lo que la app está haciendo y que una actualización no puede interrumpir.
pub trait Activity {
    /// Motivos por los que ahora no es un buen momento (vacío: se puede).
    fn busy(&self) -> Vec<String>;
}

#[derive(Debug, PartialEq)]
pub enum Readiness { Ready, Wait(Vec<String>) }

pub fn readiness(a: &dyn Activity) -> Readiness {
    let r = a.busy();
    if r.is_empty() { Readiness::Ready } else { Readiness::Wait(r) }
}

/// Argumentos del instalador para una actualización: sin preguntas, esperando a que 939 Host termine
/// de cerrarse (hasta 2 minutos) y volviéndolo a abrir al final. El instalador conserva los datos.
pub const INSTALLER_ARGS: [&str; 3] = ["/S", "/ESPERAR", "/REABRIR"];

/// Vuelve a comprobar el archivo justo antes de ejecutarlo (pudo cambiar en disco desde la descarga).
pub async fn verify_before_launch(path: &Path, a: &Artifact) -> Result<(), DownloadError> {
    let len = tokio::fs::metadata(path).await.map_err(|e| DownloadError::Io(e.to_string()))?.len();
    if len != a.size || file_sha256(path).await? != a.sha256 { return Err(DownloadError::Mismatch); }
    Ok(())
}
