//! Errores de la aplicación con mensajes comprensibles para el usuario.

use serde::Serialize;
use std::fmt;

/// Error que viaja hasta la interfaz: un código estable y un mensaje en español.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), detail: None }
    }
    pub fn with_detail(mut self, detail: impl fmt::Display) -> Self {
        self.detail = Some(detail.to_string());
        self
    }
    pub fn not_found(what: &str) -> Self {
        Self::new("not_found", format!("No se encontró {what}."))
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }
    pub fn busy(message: impl Into<String>) -> Self {
        Self::new("busy", message)
    }
    pub fn io(context: &str, err: std::io::Error) -> Self {
        let hint = match err.kind() {
            std::io::ErrorKind::PermissionDenied => " Windows negó el permiso; revisá que el archivo no esté abierto en otro programa.",
            std::io::ErrorKind::NotFound => " El archivo o la carpeta no existe.",
            _ => "",
        };
        Self::new("io", format!("{context}.{hint}")).with_detail(err)
    }
    pub fn network(context: &str, err: impl fmt::Display) -> Self {
        Self::new("network", format!("{context}. Revisá tu conexión a internet e intentá de nuevo.")).with_detail(err)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(d) => write!(f, "{} ({d})", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::new("data", "Un archivo de configuración está dañado o tiene un formato inesperado.").with_detail(e)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::network("No se pudo completar la descarga", e)
    }
}

impl From<zip::result::ZipError> for AppError {
    fn from(e: zip::result::ZipError) -> Self {
        AppError::new("archive", "No se pudo leer o escribir un archivo comprimido.").with_detail(e)
    }
}
