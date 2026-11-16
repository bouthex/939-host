//! Modelo de datos de un servidor lógico de 939 Host.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Loader {
    Vanilla,
    Fabric,
    Forge,
    #[serde(rename = "neoforge")]
    NeoForge,
}

impl Loader {
    pub fn name(&self) -> &'static str {
        match self { Loader::Vanilla => "Vanilla", Loader::Fabric => "Fabric", Loader::Forge => "Forge", Loader::NeoForge => "NeoForge" }
    }
    pub fn uses_installer(&self) -> bool { matches!(self, Loader::Forge | Loader::NeoForge) }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JvmProfile {
    /// G1 con flags estilo Aikar: el valor recomendado para heaps chicos y medianos.
    Balanced,
    /// ZGC generacional: pausas mínimas, más memoria de margen.
    LowLatency,
    /// Solo los flags que escriba el usuario.
    Custom,
}

/// Identidad permanente del servidor lógico (preparación para el cambio de anfitrión).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerIdentity {
    /// Clave pública ed25519 en hexadecimal: *es* el servidor, no depende de ninguna PC.
    pub public_key: String,
    /// Código para compartir, derivado de la clave pública.
    pub access_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub mc_version: String,
    pub loader: Loader,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub java_major: u32,
    /// Java que pide esta versión de Minecraft (según Mojang).
    #[serde(default)]
    pub java_required: u32,
    /// Java elegido a mano (None = automático, el recomendado).
    #[serde(default)]
    pub java_choice: Option<u32>,

    // Rendimiento
    pub memory_mb: u32,
    pub jvm_profile: JvmProfile,
    #[serde(default)]
    pub extra_jvm_args: String,

    // Red y jugadores
    pub port: u16,
    pub max_players: u32,
    pub motd: String,
    pub online_mode: bool,
    pub whitelist: bool,

    // Mundo
    pub difficulty: String,
    pub gamemode: String,
    pub hardcore: bool,
    pub pvp: bool,
    #[serde(default)]
    pub level_seed: String,
    #[serde(default = "default_level_type")]
    pub level_type: String,
    pub view_distance: u32,
    pub simulation_distance: u32,
    pub spawn_protection: u32,
    pub allow_flight: bool,
    #[serde(default)]
    pub enable_command_block: bool,

    // Guardado y relevo (intervalos en minutos)
    #[serde(default = "default_interval")]
    pub sync_interval_min: u32,
    #[serde(default = "default_interval")]
    pub restore_interval_min: u32,

    // Estado de la instalación
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub install_error: Option<String>,
    #[serde(default)]
    pub eula_accepted_at: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub identity: ServerIdentity,

    /// Claves de server.properties que la interfaz no administra directamente.
    #[serde(default)]
    pub extra_properties: BTreeMap<String, String>,
}

fn default_level_type() -> String { "minecraft:normal".into() }
fn default_interval() -> u32 { 15 }

/// Datos que manda el asistente de creación.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateServerInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub mc_version: String,
    pub loader: Loader,
    pub memory_mb: u32,
    pub difficulty: String,
    pub gamemode: String,
    pub max_players: u32,
    pub port: u16,
    pub online_mode: bool,
    pub whitelist: bool,
    pub view_distance: u32,
    pub simulation_distance: u32,
    #[serde(default)]
    pub level_seed: String,
    pub accept_eula: bool,
    #[serde(default)]
    pub import_world_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

/// Resumen que muestra la interfaz.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub config: ServerConfig,
    pub status: RunStatus,
    pub players: Vec<String>,
    pub started_at: Option<String>,
    pub memory_used_mb: Option<u64>,
    pub mspt: Option<f64>,
    pub mspt_p95: Option<f64>,
    pub last_error: Option<String>,
    pub needs_restart: bool,
    pub installing: bool,
    pub world_size_mb: f64,
    /// Mundo congelado con /tick freeze.
    pub paused: bool,
    /// Minecraft está guardando.
    pub saving: bool,
    /// Reinicio en curso.
    pub restarting: bool,
    /// Etapa del arranque en palabras.
    pub phase: Option<String>,
    /// Cuenta regresiva antes de detener o reiniciar.
    pub countdown: Option<crate::process::Countdown>,
    /// La versión permite pausar (/tick freeze, 1.20.3+).
    pub pause_supported: bool,
    /// Conexión real comprobada: "checking" | "ok" | "fail".
    pub reach: Option<String>,
    pub reach_info: Option<crate::reach::StatusInfo>,
    pub reach_error: Option<String>,
    /// Último intento fallido de entrar, explicado.
    pub join_issue: Option<crate::reach::JoinIssue>,
    /// Cantidad de .jar en la carpeta de mods (el juego de cada jugador tiene que coincidir).
    pub mods_count: usize,
}
