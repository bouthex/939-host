//! Apagado, reinicio y cierre de sesión de Windows con servidores encendidos.
//!
//! Windows avisa a cada ventana con `WM_QUERYENDSESSION` antes de cerrar la sesión. Si hay
//! servidores encendidos, 939 Host:
//! 1. le pide a Windows que espere, con un motivo visible («Guardando el mundo de Minecraft…»)
//!    mediante `ShutdownBlockReasonCreate` (la pantalla de apagado lo muestra y la persona siempre
//!    puede elegir «Apagar de todos modos»);
//! 2. hace el mismo cierre ordenado que «Salir»: pasa o suelta los servidores compartidos y detiene
//!    cada servidor con `stop` (Minecraft guarda todo);
//! 3. quita el motivo y termina: Windows sigue con el apagado solo.
//!
//! Nunca se bloquea indefinidamente: a los 3 minutos se quita el motivo pase lo que pase.
//! Sin servidores encendidos, 939 Host no demora el apagado.
//!
//! La ventana que recibe el aviso es invisible, sin interfaz, en su propio hilo (la ventana
//! principal puede estar cerrada mientras 939 Host sigue en la bandeja).

use tauri::AppHandle;

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tauri::AppHandle;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
    use windows_sys::Win32::System::Threading::SetProcessShutdownParameters;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
    static HWND_V: AtomicUsize = AtomicUsize::new(0);
    static BLOCKING: AtomicBool = AtomicBool::new(false);
    pub const CLASS: &str = "939HostSesion";

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

    pub fn start(app: AppHandle) {
        let _ = APP.set(app);
        std::thread::spawn(|| unsafe {
            // Que Windows le avise a 939 Host antes que a la mayoría de los programas (más tiempo
            // para guardar). 0x3FF es el nivel más alto permitido para aplicaciones.
            SetProcessShutdownParameters(0x3FF, 0);
            let class = wide(CLASS);
            let hinst = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                style: 0, lpfnWndProc: Some(wndproc), cbClsExtra: 0, cbWndExtra: 0, hInstance: hinst,
                hIcon: std::ptr::null_mut(), hCursor: std::ptr::null_mut(), hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(), lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&wc);
            let title = wide("939 Host");
            // Ventana de nivel superior pero invisible (las ventanas «solo mensajes» no reciben el aviso).
            let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0,
                std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null());
            if hwnd.is_null() { return; }
            HWND_V.store(hwnd as usize, Ordering::SeqCst);
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_QUERYENDSESSION => {
                let Some(app) = APP.get() else { return 1 };
                if BLOCKING.load(Ordering::SeqCst) { return 0; }
                let active = crate::active_servers(app);
                if active.is_empty() { return 1; }
                let reason = wide(if active.len() == 1 { "Guardando el mundo de Minecraft y deteniendo el servidor…" } else { "Guardando los mundos de Minecraft y deteniendo los servidores…" });
                ShutdownBlockReasonCreate(hwnd, reason.as_ptr());
                BLOCKING.store(true, Ordering::SeqCst);
                let cause = if (lp as u32) & ENDSESSION_LOGOFF != 0 { "cierre-de-sesion" } else { "apagado-windows" };
                crate::begin_quit_for_session(app, cause);
                // Tope: nunca se demora el apagado más de 3 minutos.
                std::thread::spawn(|| { std::thread::sleep(std::time::Duration::from_secs(180)); done(); });
                0
            }
            WM_ENDSESSION => {
                // wp != 0: la sesión termina ya (todo listo o «Apagar de todos modos»).
                let _ = wp;
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    /// El cierre ordenado terminó: Windows puede seguir.
    pub fn done() {
        if BLOCKING.swap(false, Ordering::SeqCst) {
            let h = HWND_V.load(Ordering::SeqCst);
            if h != 0 { unsafe { ShutdownBlockReasonDestroy(h as HWND); } }
        }
    }
}

#[cfg(windows)]
pub fn start(app: AppHandle) { imp::start(app) }
#[cfg(windows)]
pub fn done() { imp::done() }

#[cfg(not(windows))]
pub fn start(_app: AppHandle) {}
#[cfg(not(windows))]
pub fn done() {}
