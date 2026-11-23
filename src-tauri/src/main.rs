// 939 Host — plataforma propia para crear, administrar y hostear servidores de Minecraft Java.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod aliases;
mod files;
mod commands;
mod network;
mod ops;
mod shutdown;
mod updates;
mod packs;
mod vigia;
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
    fn server_changed(&self, id: &str) {
        let _ = self.0.emit("server-changed", IdPayload { id });
        // Si esta PC aloja el servidor de un grupo, avisa ya a las demás PCs (estado en vivo).
        let (app, id) = (self.0.clone(), id.to_string());
        tauri::async_runtime::spawn(async move { if let Some(n) = app.state::<network::Cluster>().try_node() { n.push_state(&id); } });
    }
    fn console_line(&self, id: &str, line: &str) { let _ = self.0.emit("console-line", LinePayload { id, line }); }
    fn install_progress(&self, p: &InstallProgress) { let _ = self.0.emit("install-progress", p); }
    fn mods_progress(&self, id: &str, message: &str, fraction: f64) { let _ = self.0.emit("mods-progress", ModsPayload { id, message, fraction }); }
    fn server_ready(&self, id: &str) {
        if settings::load().start_sound { host939_core::sound::play_start_chime(); }
        let _ = self.0.emit("server-ready", IdPayload { id });
    }
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

/// Argumentos de WebView2. Además de los que Tauri usa por defecto, se apagan servicios de
/// Edge que una app de escritorio local no usa y que generan trabajo y tráfico en segundo plano
/// (actualización de componentes, red en segundo plano, traducción, sugerencias). La interfaz es
/// una sola página: un único proceso de render alcanza.
fn browser_args() -> String {
    #[cfg_attr(not(feature = "pruebas"), allow(unused_mut))]
    let mut a = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Translate,OptimizationHints,MediaRouter,msEdgeTranslate,AutofillServerCommunication         --disable-background-networking --disable-component-update --disable-sync --no-pings --renderer-process-limit=1");
    // Puerto de depuración de WebView2 solo en la compilación de prueba de la CI (característica
    // `pruebas`): la app que se entrega no lo abre aunque exista la variable de entorno.
    #[cfg(feature = "pruebas")]
    if let Some(port) = std::env::var("H939_E2E_DEVTOOLS_PORT").ok().and_then(|p| p.parse::<u16>().ok()) { a.push_str(&format!(" --remote-debugging-port={port}")); }
    a
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    process::set_metrics_wanted(true);
    let _ = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .additional_browser_args(&browser_args())
        .title("939 Host")
        .inner_size(1280.0, 820.0)
        .min_inner_size(1024.0, 680.0)
        .center()
        .background_color(theme_background(None))
        .build();
}

/// Nombres de los servidores encendidos (o iniciando o guardando) en esta PC.
fn active_servers(app: &AppHandle) -> Vec<String> {
    app.state::<process::Manager>().active_ids().into_iter()
        .map(|id| host939_core::instance::load(&id).map(|c| c.name).unwrap_or(id)).collect()
}

fn tray_tooltip(app: &AppHandle, text: &str) {
    if let Some(t) = app.tray_by_id("main-tray") { let _ = t.set_tooltip(Some(text)); }
}

fn list_names(v: &[String]) -> String {
    match v.len() { 0 => String::new(), 1 => v[0].clone(), _ => format!("{} y {}", v[..v.len() - 1].join(", "), v[v.len() - 1]) }
}

/// Salida completa: pasa o guarda los servidores de forma ordenada y termina. La ventana se
/// cierra enseguida (antes quedaba abierta y parecía colgada mientras se guardaba); el cierre
/// sigue en segundo plano, con el estado en el ícono de la bandeja.
pub fn begin_quit(app: &AppHandle) { quit_inner(app, None) }

/// Windows se apaga, reinicia o cierra la sesión con servidores encendidos (ver `shutdown`).
pub fn begin_quit_for_session(app: &AppHandle, cause: &'static str) { quit_inner(app, Some(cause)) }

fn quit_inner(app: &AppHandle, session: Option<&'static str>) {
    if QUITTING.swap(true, Ordering::SeqCst) { return; }
    let _ = app.emit("app-quitting", ());
    if let Some(w) = app.get_webview_window("main") { let _ = w.destroy(); }
    tray_tooltip(app, "939 Host — cerrando: guardando los servidores…");
    let app = app.clone();
    std::thread::spawn(move || {
        // Primero el grupo: cada servidor que hostea esta PC pasa a otra PC encendida o se
        // suelta con el último estado copiado. Después se detiene lo que quede.
        let started = std::time::Instant::now();
        let active: Vec<String> = app.state::<process::Manager>().active_ids();
        for line in network::quit_blocking(&app) {
            let _ = app.emit("group-note", serde_json::json!({ "id": "", "message": line }));
        }
        app.state::<process::Manager>().stop_all();
        // Apagado de Windows: queda anotado qué pasó con cada servidor (se cuenta al volver a abrir).
        if let Some(cause) = session {
            let mgr = app.state::<process::Manager>();
            for id in &active {
                let outcome = if mgr.is_active(id) || mgr.last_stop_forced(id) { "forzado" } else { "guardado" };
                host939_core::guardian::write_record(id, &host939_core::guardian::LastClose { at: chrono::Local::now().to_rfc3339(), cause: cause.into(), outcome: outcome.into(), seconds: started.elapsed().as_secs() });
            }
            shutdown::done();
        }
        app.exit(0);
    });
}

/// «Salir» desde el ícono de la bandeja. Con servidores encendidos pregunta (ventana nativa,
/// aparece al instante aunque la interfaz esté cerrada). Si ya se está cerrando, ofrece forzarlo.
fn tray_quit(app: &AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let app = app.clone();
    if QUITTING.load(Ordering::SeqCst) {
        std::thread::spawn(move || {
            let force = app.dialog().message("939 Host todavía está guardando los servidores. Si cerrás ahora, se puede perder lo que no llegó a guardarse.")
                .title("Cerrando 939 Host").kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom("Cerrar igual".into(), "Esperar".into())).blocking_show();
            if force { app.exit(0); }
        });
        return;
    }
    let active = active_servers(&app);
    if active.is_empty() { begin_quit(&app); return; }
    std::thread::spawn(move || {
        let what = if active.len() == 1 { format!("{} está encendido", active[0]) } else { format!("{} están encendidos", list_names(&active)) };
        let ok = app.dialog().message(format!("{what}. Si salís, se guarda y se detiene (si es compartido y lo activaste, pasa a otra PC del grupo).\n\nPara que siga funcionando, cerrá solo la ventana: 939 Host queda en la bandeja."))
            .title("Salir de 939 Host").kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Salir y detener".into(), "Cancelar".into())).blocking_show();
        if ok { begin_quit(&app); }
    });
}

/// Hilo de fondo: mientras haya servidores encendidos, Windows no suspende la PC (si está
/// activado) y el ícono de la bandeja dice qué sigue funcionando.
fn start_background_watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut awake = None;
        let mut last_tip = String::new();
        loop {
            // Solo lo que está en memoria; los ajustes se leen únicamente con servidores encendidos.
            let ids = app.state::<process::Manager>().active_ids();
            let want = !ids.is_empty() && settings::load().keep_awake_while_hosting;
            if awake != Some(want) { hardware::keep_awake(want); awake = Some(want); }
            if !QUITTING.load(Ordering::SeqCst) {
                let tip = match ids.len() {
                    0 => "939 Host".to_string(),
                    1 => format!("939 Host — {} encendido", active_servers(&app).first().cloned().unwrap_or_default()),
                    n => format!("939 Host — {n} servidores encendidos"),
                };
                // El ícono se actualiza solo si el texto cambió (no una llamada al Explorador cada 10 s).
                if tip != last_tip { tray_tooltip(&app, &tip); last_tip = tip; }
            }
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
}

fn data_root() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("939Host")
}

/// Aviso para la interfaz si una migración de datos falló y se restauró el respaldo.
pub static DATA_NOTICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Datos de una versión más nueva de 939 Host: se avisa y se cierra sin tocar nada.
fn newer_data_and_exit(found: u32, supported: u32) -> ! {
    let msg = format!("Los datos de 939 Host de esta PC son de una versión más nueva (formato {found}; esta versión entiende hasta el {supported}).\n\nPara no dañarlos, esta versión no se abre. Instalá la versión más nueva de 939 Host o, si volviste atrás a propósito, restaurá el respaldo de la carpeta «respaldos-datos».");
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        MessageBoxW(std::ptr::null_mut(), w(&msg).as_ptr(), w("939 Host").as_ptr(), MB_OK | MB_ICONWARNING);
    }
    #[cfg(not(windows))]
    eprintln!("{msg}");
    std::process::exit(3)
}

fn main() {
    // Guardián de los servidores (ver host939_core::guardian): sin ventana, sin red, antes de la
    // comprobación de instancia única.
    if std::env::args().nth(1).as_deref() == Some("--guardian") { host939_core::guardian::run(); }
    // Vigía de una actualización (ver vigia.rs): sin ventana, antes de la instancia única.
    if std::env::args().nth(1).as_deref() == Some("--vigilar-actualizacion") {
        let a: Vec<String> = std::env::args().collect();
        let (Some(state), Some(pid)) = (a.get(2), a.get(3).and_then(|p| p.parse().ok())) else { std::process::exit(2) };
        vigia::run(state.into(), pid);
    }
    // Solo pruebas: una versión nueva que no abre, para comprobar la vuelta atrás automática.
    #[cfg(feature = "pruebas")]
    if std::env::args().any(|a| a == "--tras-actualizar") && std::env::var_os("H939_PRUEBA_FALLAR_TRAS_ACTUALIZAR").is_some() { std::process::exit(3); }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // La ventana se crea acá (no desde la configuración) para poder habilitar, solo en las
            // pruebas automáticas de Windows, el puerto de depuración de WebView2.
            let wcfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned().expect("ventana main");
            tauri::WebviewWindowBuilder::from_config(app.handle(), &wcfg)?.additional_browser_args(&browser_args()).build()?;
            paths::init(data_root()).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            // Formato de los datos: migra con respaldo, o no toca nada si son de una versión más nueva.
            match host939_core::datos::ensure() {
                host939_core::datos::Outcome::Newer { found, supported } => {
                    newer_data_and_exit(found, supported);
                }
                host939_core::datos::Outcome::Restored { error, .. } => {
                    *DATA_NOTICE.lock().unwrap() = Some(format!("No se pudieron actualizar los datos al formato de esta versión ({error}). Se restauró el respaldo y todo quedó como estaba; avisanos para revisarlo."));
                }
                _ => {}
            }
            // Si 939 Host termina de golpe, el guardián detiene cada servidor guardando el mundo.
            if let Ok(exe) = std::env::current_exe() { host939_core::guardian::configure(exe); }
            // Apagado y cierre de sesión de Windows con servidores encendidos.
            shutdown::start(app.handle().clone());
            // Papelera: lo que lleva más de 30 días se borra al abrir y después cada 6 horas.
            // También los temporales de operaciones cortadas (solo los de más de 6 horas).
            std::thread::spawn(|| loop {
                let _ = host939_core::trash::purge_expired();
                let _ = host939_core::storage::clean_temps();
                std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
            });
            let node = identity::load_or_create_node().map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            app.manage(node);
            app.manage(process::Manager::new(Arc::new(TauriEvents(app.handle().clone()))));
            app.manage(hardware::Probe::new());
            app.manage(aliases::Aliases::default());
            {
                let h = app.handle().clone();
                // Cada 30 s: las direcciones que llegan del grupo (o cambian) quedan escuchando sin reiniciar.
                tauri::async_runtime::spawn(async move { tokio::time::sleep(std::time::Duration::from_secs(3)).await; loop { aliases::sync(&h).await; tokio::time::sleep(std::time::Duration::from_secs(30)).await; } });
            }
            app.manage(network::Cluster::default());
            app.manage(ops::Ops::default());
            // Actualizaciones: consulta enseguida y cada 6 horas (la apertura no espera).
            updates::start(app.handle().clone());
            // Entrega local de resource packs propios al Minecraft de esta PC (127.0.0.1:39393).
            packs::start_local_server(app.handle().clone());

            let open = MenuItem::with_id(app, "open", "Abrir 939 Host", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Salir de 939 Host…", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("939 Host")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main(app),
                    "quit" => tray_quit(app),
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
            start_background_watch(app.handle().clone());
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
            if let WindowEvent::Destroyed = event {
                if window.label() == "main" { process::set_metrics_wanted(false); }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if QUITTING.load(Ordering::SeqCst) { return; }
                // Cerrar la ventana nunca detiene un servidor encendido: la ventana se destruye
                // (libera la memoria de la interfaz) y 939 Host sigue en la bandeja con los
                // servidores y la red del grupo. Salir del todo es explícito (bandeja → Salir).
                let app = window.app_handle();
                let s = settings::load();
                let active = active_servers(app);
                if active.is_empty() {
                    if !s.close_to_tray { api.prevent_close(); begin_quit(app); }
                    return;
                }
                if !s.background_hint_shown {
                    // La primera vez se explica dentro de la ventana; la interfaz la cierra después.
                    api.prevent_close();
                    let _ = app.emit("background-hint", serde_json::json!({ "servers": active }));
                }
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
            commands::storage_usage,
            commands::storage_protect,
            commands::storage_verify,
            commands::storage_enforce,
            aliases::public_address_get,
            aliases::system_cleanup,
            aliases::public_address_set,
            aliases::external_status,
            aliases::external_set,
            aliases::entrance_consent,
            aliases::entrance_port_set,
            v3::modpack_detail,
            v3::cf_project,
            v3::modpack_contents,
            v3::modpack_apply,
            commands::ops_running,
            commands::op_cancel,
            commands::play_start_sound,
            commands::last_close_notes,
            commands::data_notice,
            updates::update_status,
            updates::update_check,
            updates::update_channel,
            updates::update_install,
            packs::rp_library,
            packs::rp_search,
            packs::rp_install,
            packs::rp_add_file,
            packs::rp_remove,
            packs::rp_set,
            packs::rp_open_folder,
            network::client_prepare_undo,
            network::group_stop_remote,
            network::client_import_prefs,
            network::minecraft_dirs,
            network::minecraft_dir_set,
            updates::update_confirm,
            updates::update_notice_seen,
            updates::update_restart_now,
            v3::image_read,
            v3::profile_photo_get,
            v3::profile_photo_set,
            v3::profile_photo_clear,
            v3::avatars,
            commands::mods_settings,
            commands::mods_set_auto_updates,
            commands::mods_remove_all,
            commands::mods_update_apply,
            commands::close_to_background,
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
            network::recovery_create, network::recovery_rotate, network::recovery_reset_request, network::recovery_reset_finish, network::recovery_use,
            network::mods_approve, network::mods_policy, network::mod_requests, network::mods_reject, network::mod_decisions,
            network::successor_set, network::succession_start, network::succession_cancel, network::succession_complete,
            network::member_remove,
            network::group_options,
            network::group_handover,
            network::group_release,
            network::group_mc_op,
            network::friends_view,
            network::friend_invite,
            network::friend_add,
            network::friend_answer,
            network::friend_remove,
            network::friend_unblock_all,
            network::friend_send,
            network::friend_chat,
            network::friend_set_available,
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
            v3::cf_status,
            v3::cf_set_key,
            v3::cf_search,
            v3::cf_plan,
            v3::cf_install,
            v3::cf_updates,
            v3::modrinth_modpacks,
            v3::modpack_prepare,
            v3::modpack_create,
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
            network::client_prepare,
            v3::profile_open,
            aliases::alias_status,
            aliases::alias_set,
            aliases::alias_apply_hosts,
            v3::server_logo_set,
            v3::server_logo_clear,
            v3::server_logo_default,
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
            v3::mod_items,
            v3::player_edit_access,
            v3::player_edit,
            v3::player_adjust,
            v3::player_kill,
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
            // Cerrar la última ventana nunca termina la app: queda en la bandeja. Tampoco durante
            // «Salir» o el apagado de Windows: ahí la ventana se cierra al instante pero el proceso
            // tiene que seguir hasta que el cierre ordenado termine y llame a `app.exit(0)` (que
            // trae código). Antes se dejaba salir y el proceso moría a mitad del guardado (lo
            // detectó la prueba de apagado simulado: tuvo que rematarlo el guardián).
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
