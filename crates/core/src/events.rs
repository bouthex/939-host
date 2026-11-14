//! Eventos que el núcleo emite. La app los reenvía a la interfaz; las pruebas los registran.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgress {
    pub id: String,
    pub message: String,
    pub fraction: f64,
    pub done: bool,
    pub error: Option<String>,
}

pub trait Events: Send + Sync + 'static {
    fn server_changed(&self, id: &str);
    fn console_line(&self, id: &str, line: &str);
    fn install_progress(&self, progress: &InstallProgress);
    fn mods_progress(&self, _id: &str, _message: &str, _fraction: f64) {}
}

/// Implementación vacía (para herramientas y pruebas que no necesitan eventos).
pub struct NoEvents;
impl Events for NoEvents {
    fn server_changed(&self, _: &str) {}
    fn console_line(&self, _: &str, _: &str) {}
    fn install_progress(&self, _: &InstallProgress) {}
}
