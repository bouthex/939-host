//! Ajustes de la aplicación y del nodo (lo que esta PC aporta al grupo).

use crate::error::AppResult;
use crate::paths;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    /// Cerrar la ventana deja 939 Host en la bandeja (los servidores siguen andando).
    pub close_to_tray: bool,
    pub node_name: String,
    /// Participar en la red del grupo (etapa 3 en adelante).
    pub participation: bool,
    pub contribute_ram_mb: u32,
    pub contribute_cpu_percent: u32,
    pub allow_hosting: bool,
    pub allow_relay: bool,
    /// Relays públicos gratuitos de iroh: solo como último recurso.
    pub allow_public_relays: bool,
    pub onboarding_done: bool,
    /// Iniciar con Windows (minimizado en la bandeja).
    pub start_with_windows: bool,
    /// Perfiles de Minecraft de esta PC con la reconexión automática activada.
    #[serde(default)]
    pub reconnect_profiles: Vec<crate::client::ClientProfile>,
    /// Símbolo que acompaña al nombre de esta PC (solo visual).
    pub node_symbol: String,
    /// Apariencia: "system" (según Windows), "light" o "dark".
    pub theme: String,
    pub reduce_motion: bool,
    /// Tamaño de letra de la consola en px (11–18).
    pub console_font_size: u32,
    /// Mostrar skins reales de los jugadores (consulta a Mojang; si no, iniciales).
    pub skins_enabled: bool,
    /// Descargar de Mojang las imágenes originales del juego (ítems, corazones, efectos) a esta PC.
    pub game_assets_enabled: bool,
    /// Mostrar los íconos e imágenes de los mods de Modrinth.
    pub mod_images_enabled: bool,
    /// Clave de la API de CurseForge de esta PC, protegida con DPAPI (`dpapi:` + base64). Nunca
    /// va al ejecutable, a la interfaz ni a los registros.
    pub curseforge_key: Option<String>,
    /// Modo privado: solo relays, ninguna otra PC del grupo ve la IP de esta.
    pub private_mode: bool,
    /// Horas hasta que vence una invitación nueva (1 a 720).
    pub invite_hours: u32,
    /// Permitir que otras PCs de la red local entren directo al servidor (sin 939 Host). Apagado,
    /// el servidor de Minecraft escucha solo en esta PC (127.0.0.1) y se juega por 939 Host.
    pub lan_access: bool,
    /// Mientras esta PC aloja un servidor encendido, Windows no la suspende (la pantalla sí se
    /// puede apagar). Un servidor no puede seguir funcionando en una PC suspendida.
    pub keep_awake_while_hosting: bool,
    /// Ya se explicó una vez que cerrar la ventana deja los servidores funcionando.
    pub background_hint_shown: bool,
    /// Sonido breve cuando un servidor termina de iniciar.
    pub start_sound: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            node_name: String::new(),
            participation: true,
            contribute_ram_mb: 0,
            contribute_cpu_percent: 80,
            allow_hosting: true,
            allow_relay: true,
            allow_public_relays: true,
            onboarding_done: false,
            start_with_windows: false,
            reconnect_profiles: vec![],
            node_symbol: String::new(),
            theme: "system".into(),
            reduce_motion: false,
            console_font_size: 13,
            skins_enabled: true,
            game_assets_enabled: true,
            mod_images_enabled: true,
            curseforge_key: None,
            private_mode: false,
            invite_hours: 24,
            lan_access: false,
            keep_awake_while_hosting: true,
            background_hint_shown: false,
            start_sound: false,
        }
    }
}

fn path() -> std::path::PathBuf { paths::root().join("settings.json") }

pub fn load() -> AppSettings { paths::read_json(&path()).unwrap_or_default() }

pub fn save(s: &AppSettings) -> AppResult<()> { paths::write_json(&path(), s) }

/// Ajustes tal como los ve la interfaz: sin secretos.
pub fn for_ui(mut s: AppSettings) -> AppSettings {
    s.curseforge_key = s.curseforge_key.as_ref().map(|_| "(guardada)".into());
    s
}

/// Guarda la clave de CurseForge protegida con DPAPI (o la borra).
pub fn set_curseforge_key(key: Option<&str>) -> AppResult<()> {
    use base64::Engine;
    let mut s = load();
    s.curseforge_key = match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => Some(format!("dpapi:{}", base64::engine::general_purpose::STANDARD.encode(crate::identity::protect_bytes(k.as_bytes())?))),
        None => None,
    };
    save(&s)
}

/// La clave de CurseForge en claro (solo para la llamada a la API). Una clave guardada en texto
/// plano por versiones anteriores se vuelve a guardar protegida.
pub fn curseforge_key() -> Option<String> {
    use base64::Engine;
    let raw = load().curseforge_key?;
    if let Some(b) = raw.strip_prefix("dpapi:") {
        let blob = base64::engine::general_purpose::STANDARD.decode(b).ok()?;
        return crate::identity::unprotect_bytes(&blob).ok().and_then(|v| String::from_utf8(v).ok());
    }
    let _ = set_curseforge_key(Some(&raw));
    Some(raw)
}

/// Da a esta PC un nombre visible y un símbolo la primera vez (o si el nombre era el del
/// equipo de Windows, que nadie eligió). Solo visual: la identidad técnica no cambia.
pub fn ensure_pc_identity() -> AppSettings {
    let mut s = load();
    let host = sysinfo::System::host_name().unwrap_or_default();
    let mut changed = false;
    if s.node_name.trim().is_empty() || (!host.is_empty() && s.node_name.trim().eq_ignore_ascii_case(host.trim())) {
        s.node_name = crate::pcname::random_name();
        changed = true;
    }
    if !crate::pcname::valid_symbol(&s.node_symbol) { s.node_symbol = crate::pcname::random_symbol(); changed = true; }
    if changed { let _ = save(&s); }
    s
}

/// Registra o quita 939 Host del inicio de Windows (clave Run del usuario actual).
pub fn apply_autostart(enabled: bool) -> AppResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let exe = std::env::current_exe().map_err(|e| crate::error::AppError::io("No se encontró el ejecutable", e))?;
        if is_store_package(&exe) {
            // En el paquete de Microsoft Store el registro está virtualizado: el inicio con Windows
            // se maneja con la tarea de inicio del paquete, que el usuario activa en Windows.
            return Err(crate::error::AppError::invalid("En la versión de Microsoft Store, activalo en Configuración de Windows → Aplicaciones → Inicio → 939 Host."));
        }
        let mut cmd = std::process::Command::new(crate::paths::windows_program("reg.exe"));
        if enabled {
            cmd.args(["add", KEY, "/v", "939 Host", "/t", "REG_SZ", "/d", &format!("\"{}\" --minimized", exe.display()), "/f"]);
        } else {
            cmd.args(["delete", KEY, "/v", "939 Host", "/f"]);
        }
        let status = cmd.creation_flags(0x0800_0000).status().map_err(|e| crate::error::AppError::io("No se pudo cambiar el inicio con Windows", e))?;
        if !status.success() && enabled {
            return Err(crate::error::AppError::new("autostart", "Windows no permitió registrar el inicio automático."));
        }
    }
    #[cfg(not(windows))]
    let _ = enabled;
    Ok(())
}

/// ¿Corre instalado desde un paquete MSIX (Microsoft Store)?
pub fn is_store_package(exe: &std::path::Path) -> bool {
    exe.to_string_lossy().to_ascii_lowercase().contains("\\windowsapps\\")
}
