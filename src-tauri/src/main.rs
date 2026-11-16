// 939 Host — plataforma propia para crear, administrar y hostear servidores de Minecraft Java.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aliases;
mod files;
mod commands;
mod network;
mod v3;

use host939_core::events::{Events, InstallProgress};
use host939_core::{hardware, identity, paths, process, settings};
use std::sync::Arc;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

static QUITTING: AtomicBool = AtomicBool::new(false);

/// Reenvía los eventos del núcleo a la interfaz.
struct TauriEvents(AppHandle);

#[derive(Clone, serde::Serialize)]
struct IdPayload<'a> { id: &'a str }
#[derive(Clone, serde::Serialize)]
struct LinePayload<'a> { id: &'a str, line: &'a str }
#[derive(Clone, serde::Serialize)]
struct ModsPayload<'a> { id: &'a str, message: &'a str, fraction: f64 }

impl Events for TauriEvents {
    fn server_changed(&self, id: &str) { let _ = self.0.emit("server-changed", IdPayload { id }); }
    fn console_line(&self, id: &str, line: &str) { let _ = self.0.emit("console-line", LinePayload { id, line }); }
    fn install_progress(&self, p: &InstallProgress) { let _ = self.0.emit("install-progress", p); }
    fn mods_progress(&self, id: &str, message: &str, fraction: f64) { let _ = self.0.emit("mods-progress", ModsPayload { id, message, fraction }); }
}

/// Fondo de la ventana según el tema elegido, para que no destelle blanco en modo oscuro.
fn theme_background(w: Option<&tauri::WebviewWindow>) -> tauri::window::Color {
    let theme = settings::load().theme;
    let dark = theme == "dark" || (theme != "light" && w.and_then(|w| w.theme().ok()) == Some(tauri::Theme::Dark));
    if dark { tauri::window::Color(9, 9, 9, 255) } else { tauri::window::Color(250, 250, 250, 255) }
}

/// Muestra la ventana principal; si se había destruido para ahorrar memoria, la vuelve a crear.
/// Minimizada: le pide a WebView2 que use la menor memoria posible (API oficial
/// `MemoryUsageTargetLevel`, ICoreWebView2_19). Al restaurar vuelve a lo normal.
fn webview_memory(window: &tauri::Window) {
    static LOW: AtomicBool = AtomicBool::new(false);
    let min = window.is_minimized().unwrap_or(false);
    if LOW.swap(min, Ordering::SeqCst) == min { return; }
    let Some(w) = window.app_handle().get_webview_window(window.label()) else { return };
    let _ = w.with_webview(move |_wv| {
        #[cfg(windows)]
        unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL};
            use windows_core::Interface;
            if let Ok(core) = _wv.controller().CoreWebView2() {
                if let Ok(c) = core.cast::<ICoreWebView2_19>() {
                    let _ = c.SetMemoryUsageTargetLevel(if min { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL });
                }
            }
        }
    });
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("939 Host")
        .inner_size(1280.0, 820.0)
        .min_inner_size(1024.0, 680.0)
        .center()
        .background_color(theme_background(None))
        .build();
}

/// Salida completa: detiene todos los servidores de forma ordenada y libera todo.
pub fn begin_quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) { return; }
    let _ = app.emit("app-quitting", ());
    let app = app.clone();
    std::thread::spawn(move || {
        // Primero el grupo: cada servidor que hostea esta PC pasa a otra PC encendida o se
        // suelta con el último estado copiado. Después se detiene lo que quede.
        for line in network::quit_blocking(&app) {
            let _ = app.emit("group-note", serde_json::json!({ "id": "", "message": line }));
        }
        app.state::<process::Manager>().stop_all();
        app.exit(0);
    });
}

fn data_root() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("939Host")
}

fn main() {
    // Ayudante con permisos de administrador para el archivo hosts: hace solo eso y termina,
    // antes de abrir ventanas o de la comprobación de instancia única.
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = aliases::helper_main(&args) { std::process::exit(code); }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // La ventana se crea acá (no desde la configuración) para poder habilitar, solo en las
            // pruebas automáticas de Windows, el puerto de depuración de WebView2.
            let wcfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned().expect("ventana main");
            let mut wb = tauri::WebviewWindowBuilder::from_config(app.handle(), &wcfg)?;
            if let Some(port) = std::env::var("H939_E2E_DEVTOOLS_PORT").ok().and_then(|p| p.parse::<u16>().ok()) {
                wb = wb.additional_browser_args(&format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port}"));
            }
            wb.build()?;
            paths::init(data_root()).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            // Papelera: lo que lleva más de 30 días se borra al abrir y después cada 6 horas.
            std::thread::spawn(|| loop {
                let _ = host939_core::trash::purge_expired();
                std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
            });
            let node = identity::load_or_create_node().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            app.manage(node);
            app.manage(process::Manager::new(Arc::new(TauriEvents(app.handle().clone()))));
            app.manage(hardware::Probe::new());
            app.manage(aliases::Aliases::default());
            {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(3)).await; aliases::sync(&h).await; });
            }
            app.manage(network::Cluster::default());

            let open = MenuItem::with_id(app, "open", "Abrir 939 Host", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Salir (detiene los servidores)", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("939 Host")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main(app),
                    "quit" => begin_quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() { tray = tray.icon(icon.clone()); }
            tray.build(app)?;
            commands::start_snapshot_scheduler(app.handle().clone());
            // Con --minimized (inicio con Windows) arranca solo en la bandeja.
            if !std::env::args().any(|a| a == "--minimized") {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_background_color(Some(theme_background(Some(&w))));
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            } else if let Some(w) = app.get_webview_window("main") {
                let _ = w.destroy();
            }
            settings::ensure_pc_identity();
            network::start(app.handle().clone());
            commands::recheck_client_reconnect();
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::Resized(_) = event { webview_memory(window); }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if QUITTING.load(Ordering::SeqCst) { return; }
                if !settings::load().close_to_tray {
                    api.prevent_close();
                    begin_quit(window.app_handle());
                }
                // Con "cerrar a la bandeja", la ventana se destruye (libera la memoria de la
                // interfaz) y la app sigue en la bandeja con los servidores andando.
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::save_settings,
            commands::list_servers,
            commands::get_server,
            commands::list_versions,
            commands::fabric_support,
            commands::loader_support,
            commands::client_profiles,
            commands::players_list,
            commands::mods_import_folder,
            commands::player_action,
            commands::world_info,
            commands::world_new,
            commands::diagnostics_report,
            commands::client_reconnect_enable,
            commands::client_reconnect_disable,
            commands::suggest_port,
            commands::create_server,
            commands::reinstall_server,
            commands::update_server,
            commands::server_properties_extra,
            commands::delete_server,
            commands::start_server,
            commands::stop_server,
            commands::kill_server,
            commands::restart_server,
            commands::send_command,
            commands::console_history,
            commands::list_backups,
            commands::create_backup,
            commands::restore_backup,
            commands::delete_backup,
            commands::import_world,
            commands::validate_world,
            commands::open_folder,
            commands::open_link,
            commands::hardware_snapshot,
            commands::hardware_live,
            commands::run_benchmark,
            commands::list_runtimes,
            commands::jvm_preview,
            commands::quit_app,
            commands::notify_ui_ready,
            commands::mods_search,
            commands::mods_versions,
            commands::mods_plan,
            commands::mods_install,
            commands::mods_list,
            commands::mods_remove,
            commands::mods_set_aside,
            commands::mods_updates,
            commands::snapshots_list,
            commands::snapshot_take,
            commands::snapshot_restore,
            commands::snapshot_delete,
            network::share_status,
            network::share_set,
            network::invite_create,
            network::invite_revoke,
            network::member_remove,
            network::group_options,
            network::group_handover,
            network::group_release,
            network::group_take_here,
            network::group_sync,
            network::remote_list,
            network::remote_join,
            network::remote_remove,
            network::pc_identity_set,
            network::pc_name_random,
            network::host_request_send,
            network::host_request_cancel,
            network::host_request_answer,
            network::host_requests_pending,
            network::servers_hosting,
            v3::pause_server,
            v3::trash_list,
            files::files_list,
            files::files_search,
            files::files_mkdir,
            files::files_delete,
            files::files_transfer,
            files::files_cancel,
            files::files_import,
            files::files_export,
            files::files_check_jar,
            files::files_open_local,
            v3::optimize_options,
            v3::optimize_set_pending,
            v3::optimize_install,
            network::group_transfer,
            network::group_leave,
            v3::profile_export,
            v3::profile_open,
            aliases::alias_status,
            aliases::alias_set,
            aliases::alias_apply_hosts,
            v3::server_logo_set,
            v3::server_logo_clear,
            v3::version_analyze,
            v3::version_last_change,
            v3::version_update,
            v3::version_rollback,
            v3::server_duplicate,
            v3::trash_restore,
            v3::trash_purge,
            v3::trash_empty,
            v3::server_check_connection,
            v3::server_clear_join_issue,
            v3::resume_server,
            v3::stop_server_warned,
            v3::restart_server_warned,
            v3::cancel_countdown,
            v3::players_online_count,
            v3::player_detail,
            v3::player_skin,
            v3::game_assets_status,
            v3::game_assets_prepare,
            v3::game_sprites,
            v3::game_names,
            v3::game_assets_clear,
            v3::repair_analyze,
            v3::repair_apply,
            v3::repair_undo,
            v3::repair_last,
            v3::mods_explore,
            v3::mods_project,
            v3::mods_import_files,
            v3::mods_link,
        ])
        .build(tauri::generate_context!())
        .expect("no se pudo iniciar 939 Host");

    app.run(|_app, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Cerrar la última ventana no termina la app: queda en la bandeja.
            if code.is_none() && !QUITTING.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
    });
}
