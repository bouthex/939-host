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
    /// "cierre-inesperado" | "apagado-windows" | "cierre-de-sesion" | "cierre-sin-ver" (corte de luz o
    /// proceso terminado junto con el guardián: lo detecta la marca `en-marcha`)
    pub cause: String,
    /// "guardado" (Minecraft confirmó el guardado y se detuvo) | "detenido" (se detuvo, sin
    /// confirmación en el registro) | "forzado" (no respondió en 10 minutos) | "sin-confirmar"
    /// (nadie vio el cierre)
    pub outcome: String,
    pub seconds: u64,
}

fn record_path(id: &str) -> PathBuf { paths::server_dir(id).join("ultimo-cierre.json") }
fn marker_path(id: &str) -> PathBuf { paths::server_dir(id).join("recuperando") }

pub fn write_record(id: &str, r: &LastClose) { let _ = paths::write_json(&record_path(id), r); }

/// Cierres sin ver ya avisados en esta ejecución (la ventana se vuelve a crear desde la bandeja).
static NOTIFIED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// Lee y quita los avisos pendientes de todos los servidores (para mostrarlos una vez).
pub fn take_records() -> Vec<(String, LastClose)> {
    let mut out = vec![];
    for id in crate::instance::list_configs().into_iter().map(|c| c.id) {
        let p = record_path(&id);
        if let Ok(r) = paths::read_json::<LastClose>(&p) {
            let _ = std::fs::remove_file(&p);
            crate::process::clear_running(&id);
            out.push((id, r));
        } else if crate::process::unconfirmed_close(&id) && NOTIFIED.lock().unwrap().insert(id.clone()) {
            // Nadie vio el cierre (corte de luz, «Finalizar árbol de procesos»): se avisa una vez.
            // La marca queda hasta el próximo encendido o traspaso, para rotular ese punto.
            let at = std::fs::metadata(crate::paths::server_dir(&id).join("en-marcha")).and_then(|m| m.modified()).ok()
                .map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339()).unwrap_or_default();
            out.push((id, LastClose { at, cause: "cierre-sin-ver".into(), outcome: "sin-confirmar".into(), seconds: 0 }));
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

/// Resultado de detener un servidor con `stop`: lo que Minecraft escribió en `logs/latest.log`
/// desde entonces y su código de salida. Una salida con código 0 después de `stop` significa que
/// Minecraft terminó su secuencia normal de cierre, que siempre guarda jugadores y mundos (desde
/// 26.3 ya no escribe «All dimensions are saved», así que el código de salida es la señal fiable).
pub fn outcome_from_log(appended: &str, exited: bool, exit_code: Option<u32>) -> &'static str {
    if !exited { return "forzado"; }
    let saved_msg = ["All dimensions are saved", "All chunks are saved", "Saved the game"].iter().any(|k| appended.contains(k));
    let clean_stop = exit_code == Some(0) && appended.contains("Stopping");
    if saved_msg || clean_stop { "guardado" } else { "detenido" }
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
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, TerminateProcess, WaitForSingleObject};

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
                let mut code: u32 = 1;
                let code = (exited && unsafe { GetExitCodeProcess(s.proc_h as HANDLE, &mut code) } != 0).then_some(code);
                let outcome = outcome_from_log(&log_since(run_dir, *from), exited, code);
                write_record(&s.id, &LastClose { at: chrono::Local::now().to_rfc3339(), cause: "cierre-inesperado".into(), outcome: outcome.into(), seconds: started.elapsed().as_secs() });
                // El estado final se publica al grupo la próxima vez que se abre 939 Host.
                if outcome != "forzado" { crate::process::mark_exit_unsaved(&s.id); }
                // El cierre ya quedó registrado (con su resultado): no es un cierre sin ver.
                crate::process::clear_running(&s.id);
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
    fn un_cierre_que_nadie_vio_se_avisa_una_vez_y_no_se_confunde_con_uno_en_curso() {
        let dir = std::env::temp_dir().join(format!("h939-guard-{}", std::process::id()));
        let _ = paths::init(dir);
        let mut b = [0u8; 8]; getrandom::getrandom(&mut b).unwrap();
        let id = format!("srv{}", u64::from_le_bytes(b));
        std::fs::create_dir_all(paths::server_dir(&id)).unwrap();
        // Configuración mínima: los avisos se buscan entre los servidores configurados.
        std::fs::write(paths::server_dir(&id).join("instance.json"), serde_json::json!({
            "id": id, "name": "x", "description": "", "mcVersion": "1.21.1", "loader": "vanilla",
            "loaderVersion": null, "javaMajor": 21, "memoryMb": 1024, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565,
            "maxPlayers": 4, "motd": "", "onlineMode": true, "whitelist": false, "difficulty": "easy", "gamemode": "survival",
            "hardcore": false, "pvp": true, "levelSeed": "", "levelType": "minecraft:normal", "viewDistance": 8, "simulationDistance": 6,
            "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15, "restoreIntervalMin": 15,
            "installed": true, "installError": null, "eulaAcceptedAt": "x", "createdAt": "x",
            "identity": { "publicKey": "a", "accessCode": "X" }, "extraProperties": {}
        }).to_string()).unwrap();
        assert!(crate::instance::list_configs().iter().any(|c| c.id == id), "la configuración de prueba no se lee");
        // Encendido en este proceso: la marca es la de ahora, no un cierre sin ver.
        crate::process::mark_running(&id);
        assert!(!crate::process::unconfirmed_close(&id));
        // Como si fuera de una ejecución anterior que se cortó: la marca quedó y nadie la vio.
        crate::process::clear_running(&id);
        std::fs::write(paths::server_dir(&id).join("en-marcha"), b"1").unwrap();
        assert!(crate::process::unconfirmed_close(&id));
        let mine = |v: Vec<(String, LastClose)>| v.into_iter().filter(|(i, _)| i == &id).collect::<Vec<_>>();
        let r = mine(take_records());
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].1.cause.as_str(), r[0].1.outcome.as_str()), ("cierre-sin-ver", "sin-confirmar"));
        assert!(mine(take_records()).is_empty(), "se avisa una sola vez");
        // La marca sigue (rotula el próximo traspaso) hasta que se usa.
        assert!(crate::process::take_unconfirmed(&id));
        assert!(!crate::process::unconfirmed_close(&id));
        // Si el guardián registró el cierre, ese registro manda y la marca se quita.
        std::fs::write(paths::server_dir(&id).join("en-marcha"), b"1").unwrap();
        write_record(&id, &LastClose { at: String::new(), cause: "cierre-inesperado".into(), outcome: "guardado".into(), seconds: 1 });
        let r = mine(take_records());
        assert_eq!(r[0].1.cause, "cierre-inesperado");
        assert!(!crate::process::unconfirmed_close(&id));
    }

    #[test]
    fn resultado_segun_el_registro() {
        let old = "[Server thread/INFO]: Stopping the server\n[Server thread/INFO]: Saving worlds\n[Server thread/INFO]: ThreadedAnvilChunkStorage: All dimensions are saved\n";
        assert_eq!(outcome_from_log(old, true, Some(1)), "guardado", "versiones viejas: el mensaje alcanza");
        // 26.3: sin «All dimensions are saved»; vale la salida normal (código 0) tras «Stopping».
        let new = "[Server thread/INFO]: Stopping server\n[Server thread/INFO]: Saving players\n[Server thread/INFO]: Saving worlds\n[Server thread/INFO]: Saving chunks for level 'ServerLevel[world]'/minecraft:overworld\n";
        assert_eq!(outcome_from_log(new, true, Some(0)), "guardado");
        assert_eq!(outcome_from_log(new, true, Some(1)), "detenido", "salida con error: sin confirmación");
        assert_eq!(outcome_from_log("", true, Some(0)), "detenido", "sin «Stopping» no se da por guardado");
        assert_eq!(outcome_from_log(old, false, None), "forzado");
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
