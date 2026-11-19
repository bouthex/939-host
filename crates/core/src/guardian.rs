//! Guardián: protege el mundo si 939 Host termina de golpe con un servidor encendido.
//!
//! Antes, Java estaba en un Job Object con `KILL_ON_JOB_CLOSE`: si 939 Host moría (Administrador
//! de tareas, un fallo), Windows cerraba Java en el acto, sin guardar. Ahora:
//!
//! - Al encender el primer servidor, 939 Host abre un proceso guardián chico (el mismo ejecutable
//!   con `--guardian`, sin ventana, sin red, sin privilegios). Le duplica tres cosas: el Job
//!   Object, el proceso de Java y la entrada de su consola.
//! - Mientras 939 Host vive, el guardián solo espera. Si 939 Host termina y algún Java sigue
//!   encendido (cierre inesperado o «Cerrar igual»), el guardián le manda `stop` por su consola
//!   (Minecraft guarda todo y se detiene, como al detenerlo normalmente), espera con la misma regla
//!   que una detención normal (hasta 10 minutos si sigue guardando) y anota qué pasó.
//! - Java nunca queda sin supervisión: el guardián termina en cuanto Java se detiene, y si el
//!   guardián también muere, el Job Object cierra Java como antes (último recurso).
//! - La próxima vez que se abre 939 Host, cuenta qué se pudo guardar (`ultimo-cierre.json`) y, si
//!   el servidor era compartido, publica ese estado final al grupo.
//!
//! Un corte de luz no se puede atajar: protegen los guardados de Minecraft y los puntos de
//! restauración (cada 15 minutos por defecto).

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Qué pasó con un servidor la última vez que 939 Host se cerró de forma no normal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastClose {
    pub at: String,
    /// "cierre-inesperado" | "apagado-windows"
    pub cause: String,
    /// "guardado" (Minecraft confirmó el guardado y se detuvo) | "detenido" (se detuvo, sin
    /// confirmación en el registro) | "forzado" (no respondió en 10 minutos)
    pub outcome: String,
    pub seconds: u64,
}

fn record_path(id: &str) -> PathBuf { paths::server_dir(id).join("ultimo-cierre.json") }
fn marker_path(id: &str) -> PathBuf { paths::server_dir(id).join("recuperando") }

pub fn write_record(id: &str, r: &LastClose) { let _ = paths::write_json(&record_path(id), r); }

/// Lee y quita los avisos pendientes de todos los servidores (para mostrarlos una vez).
pub fn take_records() -> Vec<(String, LastClose)> {
    let mut out = vec![];
    for id in crate::instance::list_configs().into_iter().map(|c| c.id) {
        let p = record_path(&id);
        if let Ok(r) = paths::read_json::<LastClose>(&p) {
            let _ = std::fs::remove_file(&p);
            out.push((id, r));
        }
    }
    out
}

/// ¿El guardián está guardando este servidor después de un cierre inesperado? (marca de menos de
/// 11 minutos: si el guardián murió, la marca vieja no bloquea para siempre).
pub fn recovering(id: &str) -> bool {
    std::fs::metadata(marker_path(id)).and_then(|m| m.modified()).ok()
        .and_then(|t| t.elapsed().ok()).map(|e| e.as_secs() < 660).unwrap_or(false)
}

/// Resultado según lo que Minecraft escribió en `logs/latest.log` desde que se le pidió `stop`.
pub fn outcome_from_log(appended: &str, exited: bool) -> &'static str {
    if !exited { return "forzado"; }
    let saved = ["All dimensions are saved", "All chunks are saved", "Saved the game", "ThreadedAnvilChunkStorage: All dimensions are saved"].iter().any(|k| appended.contains(k));
    if saved { "guardado" } else { "detenido" }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn log_len(run_dir: &Path) -> u64 { std::fs::metadata(run_dir.join("logs/latest.log")).map(|m| m.len()).unwrap_or(0) }

#[cfg_attr(not(windows), allow(dead_code))]
fn log_since(run_dir: &Path, from: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(run_dir.join("logs/latest.log")) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    // Si Minecraft rotó el archivo (más corto que antes), se lee desde el principio.
    let start = if len < from { 0 } else { from };
    if f.seek(SeekFrom::Start(start)).is_err() { return String::new(); }
    let mut buf = Vec::new();
    let _ = f.take(4 * 1024 * 1024).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/* ───────────────────────── lado de 939 Host ───────────────────────── */

#[cfg(windows)]
mod host {
    use once_cell::sync::OnceCell;
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::Mutex;
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    static EXE: OnceCell<PathBuf> = OnceCell::new();
    static PIPE: Mutex<Option<(Child, ChildStdin)>> = Mutex::new(None);

    pub fn configure(exe: PathBuf) { let _ = EXE.set(exe); }

    unsafe fn dup_into(target: HANDLE, h: HANDLE) -> Option<usize> {
        let mut out: HANDLE = std::ptr::null_mut();
        let ok = DuplicateHandle(GetCurrentProcess(), h, target, &mut out, 0, 0, DUPLICATE_SAME_ACCESS);
        (ok != 0).then_some(out as usize)
    }

    /// Abre el guardián si hace falta. Devuelve la tubería para mandarle órdenes.
    fn ensure(g: &mut Option<(Child, ChildStdin)>) -> Option<()> {
        if let Some((c, _)) = g.as_mut() { if matches!(c.try_wait(), Ok(None)) { return Some(()); } }
        *g = None;
        let exe = EXE.get()?;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new(exe).arg("--guardian").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW).spawn().ok()?;
        let mut stdin = child.stdin.take()?;
        let job = crate::process::job_handle()?;
        let job = unsafe { dup_into(child.as_raw_handle() as HANDLE, job as HANDLE)? };
        writeln!(stdin, "root\t{}", crate::paths::root().display()).ok()?;
        writeln!(stdin, "job\t{job}").ok()?;
        stdin.flush().ok()?;
        *g = Some((child, stdin));
        Some(())
    }

    /// Le pasa al guardián el Java recién encendido y la entrada de su consola.
    pub fn watch(id: &str, java: &Child, java_stdin: &ChildStdin) {
        if EXE.get().is_none() { return; }
        let mut g = PIPE.lock().unwrap();
        if ensure(&mut g).is_none() { return; }
        let Some((gc, gin)) = g.as_mut() else { return };
        let target = gc.as_raw_handle() as HANDLE;
        let (Some(p), Some(i)) = (unsafe { dup_into(target, java.as_raw_handle() as HANDLE) }, unsafe { dup_into(target, java_stdin.as_raw_handle() as HANDLE) }) else { return };
        let _ = writeln!(gin, "srv\t{id}\t{p}\t{i}").and_then(|_| gin.flush());
    }

    /// El servidor se detuvo normalmente: el guardián suelta sus copias de los handles.
    pub fn release(id: &str) {
        let mut g = PIPE.lock().unwrap();
        if let Some((_, gin)) = g.as_mut() { let _ = writeln!(gin, "end\t{id}").and_then(|_| gin.flush()); }
    }
}

#[cfg(not(windows))]
mod host {
    pub fn configure(_exe: std::path::PathBuf) {}
    pub fn watch(_id: &str, _java: &std::process::Child, _stdin: &std::process::ChildStdin) {}
    pub fn release(_id: &str) {}
}

pub use host::{configure, release, watch};

/* ───────────────────────── el guardián ───────────────────────── */

/// Punto de entrada de `939 Host.exe --guardian`. No vuelve.
#[cfg(windows)]
pub fn run() -> ! {
    use std::io::{BufRead, Write};
    use std::os::windows::io::FromRawHandle;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

    struct Srv { id: String, proc_h: usize, stdin_h: usize }
    let mut servers: Vec<Srv> = vec![];
    let mut job: Option<usize> = None;
    let stdin = std::io::stdin();
    // Mientras 939 Host vive, la tubería está abierta. EOF = 939 Host terminó (como sea).
    for line in stdin.lock().lines().map_while(Result::ok) {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            ["root", dir] => { let _ = paths::init(PathBuf::from(dir)); }
            ["job", h] => job = h.parse().ok(),
            ["srv", id, p, i] => {
                if let (Ok(p), Ok(i)) = (p.parse(), i.parse()) {
                    servers.retain(|s| { if s.id == *id { unsafe { CloseHandle(s.proc_h as HANDLE); CloseHandle(s.stdin_h as HANDLE); } false } else { true } });
                    servers.push(Srv { id: id.to_string(), proc_h: p, stdin_h: i });
                }
            }
            ["end", id] => servers.retain(|s| { if s.id == *id { unsafe { CloseHandle(s.proc_h as HANDLE); CloseHandle(s.stdin_h as HANDLE); } false } else { true } }),
            _ => {}
        }
    }
    let alive = |h: usize| unsafe { WaitForSingleObject(h as HANDLE, 0) == WAIT_TIMEOUT };
    let running: Vec<Srv> = servers.into_iter().filter(|s| alive(s.proc_h)).collect();
    // Se detienen todos a la vez (cada uno guarda su mundo).
    let mut work = vec![];
    for s in running {
        let run_dir = paths::server_run_dir(&s.id);
        let _ = std::fs::write(marker_path(&s.id), std::process::id().to_string());
        let from = log_len(&run_dir);
        let mut pipe = unsafe { std::fs::File::from_raw_handle(s.stdin_h as _) };
        let _ = pipe.write_all(b"stop\n").and_then(|_| pipe.flush());
        work.push((s, run_dir, from, Instant::now(), Instant::now(), 0u64, pipe));
    }
    loop {
        let mut pending = false;
        for (s, run_dir, from, started, last_change, last_len, _pipe) in work.iter_mut() {
            if s.proc_h == 0 { continue; }
            let w = unsafe { WaitForSingleObject(s.proc_h as HANDLE, 1000) };
            let len = log_len(run_dir);
            if len != *last_len { *last_len = len; *last_change = Instant::now(); }
            let exited = w == WAIT_OBJECT_0;
            let give_up = !exited && crate::process::stop_gives_up(started.elapsed(), last_change.elapsed());
            if exited || give_up {
                if give_up { unsafe { TerminateProcess(s.proc_h as HANDLE, 1); } }
                let outcome = outcome_from_log(&log_since(run_dir, *from), exited);
                write_record(&s.id, &LastClose { at: chrono::Local::now().to_rfc3339(), cause: "cierre-inesperado".into(), outcome: outcome.into(), seconds: started.elapsed().as_secs() });
                // El estado final se publica al grupo la próxima vez que se abre 939 Host.
                if outcome != "forzado" { crate::process::mark_exit_unsaved(&s.id); }
                let _ = std::fs::remove_file(marker_path(&s.id));
                unsafe { CloseHandle(s.proc_h as HANDLE); }
                s.proc_h = 0;
            } else {
                pending = true;
            }
        }
        if !pending { break; }
        std::thread::sleep(Duration::from_millis(200));
    }
    if let Some(j) = job { unsafe { CloseHandle(j as HANDLE); } }
    std::process::exit(0)
}

#[cfg(not(windows))]
pub fn run() -> ! { std::process::exit(0) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resultado_segun_el_registro() {
        let saved = "[Server thread/INFO]: Stopping the server\n[Server thread/INFO]: Saving worlds\n[Server thread/INFO]: ThreadedAnvilChunkStorage: All dimensions are saved\n";
        assert_eq!(outcome_from_log(saved, true), "guardado");
        assert_eq!(outcome_from_log("[Server thread/INFO]: Stopping the server\n", true), "detenido");
        assert_eq!(outcome_from_log(saved, false), "forzado");
    }

    #[test]
    fn avisos_y_marca_de_recuperacion() {
        let _ = paths::init(std::env::temp_dir().join(format!("939host-guardian-{}", std::process::id())));
        let id = "g1";
        std::fs::create_dir_all(paths::server_dir(id)).unwrap();
        assert!(!recovering(id));
        std::fs::write(marker_path(id), "1").unwrap();
        assert!(recovering(id), "marca reciente: se está guardando");
        let _ = std::fs::remove_file(marker_path(id));
        let dir = paths::server_run_dir(id).join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.log"), "viejo\n").unwrap();
        let from = log_len(&paths::server_run_dir(id));
        std::fs::write(dir.join("latest.log"), "viejo\nAll dimensions are saved\n").unwrap();
        assert!(log_since(&paths::server_run_dir(id), from).contains("All dimensions"));
        assert!(!log_since(&paths::server_run_dir(id), from).contains("viejo"));
    }
}
