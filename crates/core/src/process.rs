//! Ciclo de vida de los procesos de servidor: arranque, consola, métricas y detención ordenada.
//!
//! Garantías:
//! - Cada servidor corre en su propio proceso Java, sin ventana de consola.
//! - Todos los procesos quedan dentro de un Job Object de Windows: si 939 Host termina por
//!   cualquier motivo, Windows los cierra. Nunca quedan servidores "huérfanos" consumiendo RAM.
//! - Un lock en la carpeta impide que el mismo mundo arranque dos veces en esta PC.

use crate::error::{AppError, AppResult};
use crate::hardware::Probe;
use crate::java;
use crate::model::{JvmProfile, Loader, RunStatus, ServerConfig};
use crate::paths;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::events::Events;

const LOG_LINES: usize = 2000;

#[derive(Default)]
pub struct Runtime {
    pub status: Option<RunStatus>,
    pub players: BTreeSet<String>,
    /// Sesiones en curso para el historial (nombre → hora de entrada).
    pub open_sessions: crate::sessions::Open,
    pub started_at: Option<chrono::DateTime<chrono::Local>>,
    pub pid: Option<u32>,
    stdin: Option<ChildStdin>,
    pub log: VecDeque<String>,
    pub last_error: Option<String>,
    pub needs_restart: bool,
    stop_requested: bool,
    pub mspt: Option<f64>,
    pub mspt_p95: Option<f64>,
    /// Ticks por segundo medidos (con `tick query` se calcula del tiempo por tick; en versiones sin
    /// ese comando, con `debug start`/`debug stop`, que existe desde siempre).
    pub tps: Option<f64>,
    metrics_supported: bool,
    /// Medición con `debug` (versiones anteriores a 1.20.3): None = sin probar todavía.
    debug_metrics: Option<bool>,
    debug_running: bool,
    suppress_until: Option<Instant>,
    pub save_counter: u64,
    pub installing: bool,
    pub busy: Option<String>,
    /// Mundo congelado con `/tick freeze` (los jugadores siguen conectados).
    pub paused: bool,
    /// Minecraft está guardando el mundo (entre "Saving the game" y "Saved the game").
    pub saving: bool,
    /// Reinicio en curso (detener + iniciar).
    pub restarting: bool,
    /// Etapa del arranque, en palabras ("Cargando el mundo · 64 %").
    pub phase: Option<String>,
    /// Cuenta regresiva activa antes de detener o reiniciar.
    pub countdown: Option<Countdown>,
    countdown_gen: u64,
    queries: Vec<Query>,
    /// Comprobación real de conexión: "checking" | "ok" | "fail" (None = sin comprobar).
    pub reach: Option<String>,
    pub reach_info: Option<crate::reach::StatusInfo>,
    pub reach_error: Option<String>,
    /// Último intento fallido de entrar, explicado.
    pub join_issue: Option<crate::reach::JoinIssue>,
    join_watch: crate::reach::JoinWatch,
    /// Contexto para explicar entradas fallidas: versión, loader, mods.
    join_ctx: (String, String, usize),
    /// El proceso terminó (detención, cierre forzado o caída) y todavía no se tomó un punto con
    /// lo que quedó en disco. En un servidor compartido, la anfitriona lo publica enseguida para
    /// que «Detener» no deje el estado final solo en esta PC.
    pub exited_unsaved: bool,
    /// Última línea que escribió el servidor (para no cortar un guardado largo).
    last_output: Option<Instant>,
    /// La última detención terminó con cierre forzado (el mundo puede no estar guardado).
    pub forced_stop: bool,
    /// Ya se avisó que este arranque quedó listo (sonido y aviso: una sola vez por arranque).
    ready_announced: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Countdown {
    /// "stop" | "restart"
    pub kind: String,
    pub ends_at: String,
    pub seconds: u32,
}

/// Consulta interna: un comando cuya respuesta no se muestra en la consola.
struct Query {
    needles: Vec<String>,
    tx: std::sync::mpsc::Sender<String>,
    until: Instant,
}

impl Runtime {
    pub fn status(&self) -> RunStatus { self.status.unwrap_or(RunStatus::Stopped) }
    pub fn is_active(&self) -> bool { matches!(self.status(), RunStatus::Starting | RunStatus::Running | RunStatus::Stopping) }
    fn write_stdin(&mut self, cmd: &str) -> AppResult<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| AppError::invalid("La consola del servidor no está disponible."))?;
        writeln!(stdin, "{cmd}").and_then(|_| stdin.flush()).map_err(|e| AppError::io("No se pudo enviar el comando", e))
    }
    fn reset_live(&mut self) {
        self.reach = None;
        self.reach_info = None;
        self.reach_error = None;
        self.join_watch = Default::default();
        self.paused = false;
        self.saving = false;
        self.phase = None;
        self.countdown = None;
        self.countdown_gen += 1;
        self.queries.clear();
    }
    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_LINES { self.log.pop_front(); }
        self.log.push_back(line);
    }
}

pub struct Manager {
    runtimes: Mutex<HashMap<String, Arc<Mutex<Runtime>>>>,
    pub events: Arc<dyn Events>,
}

static RE_DONE: Lazy<Regex> = Lazy::new(|| Regex::new(r"Done \([0-9.,]+s\)!").unwrap());
static RE_AVG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)average time per tick:\s*([0-9.]+)\s*ms").unwrap());
static RE_P95: Lazy<Regex> = Lazy::new(|| Regex::new(r"P95:\s*([0-9.]+)\s*ms").unwrap());
static RE_BIND: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)failed to bind to port").unwrap());
const TICK_PATTERNS: &[&str] = &["The game is running normally", "Target tick rate", "Average time per tick", "Percentiles", "The game is frozen", "The game is sprinting", "Unknown or incomplete command", "<--[HERE]",
    "Started debug profiling", "Started tick profiling", "Stopped debug profiling", "Stopped tick profiling", "profiling", "Profiler"];
static RE_DEBUG_TPS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)stopped (?:debug |tick )?profiling after ([0-9.,]+) seconds and (\d+) ticks \(([0-9.,]+) ticks? per second\)").unwrap());

/// Borra los perfiles que deja `debug stop` (solo los de 939 Host: los de los últimos minutos).
fn clean_debug_profiles(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        let recent = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d < Duration::from_secs(300)).unwrap_or(false);
        if recent && (n.starts_with("profile-results") || n.starts_with("profiling") || n.ends_with("-server.txt") || n.ends_with("-server")) {
            let p = e.path();
            if p.is_dir() { let _ = std::fs::remove_dir_all(&p); } else { let _ = std::fs::remove_file(&p); }
        }
    }
}

impl Manager {
    pub fn new(events: Arc<dyn Events>) -> Self {
        Self { runtimes: Mutex::new(HashMap::new()), events }
    }

    pub fn changed(&self, id: &str) { self.events.server_changed(id); }

    pub fn runtime(&self, id: &str) -> Arc<Mutex<Runtime>> {
        self.runtimes.lock().unwrap().entry(id.to_string()).or_default().clone()
    }

    /// ¿Este servidor está encendido, arrancando o deteniéndose?
    pub fn is_active(&self, id: &str) -> bool { self.runtime(id).lock().unwrap().is_active() }

    /// ¿La última detención tuvo que cerrar Java a la fuerza (no terminó de guardar a tiempo)?
    pub fn last_stop_forced(&self, id: &str) -> bool { self.runtime(id).lock().unwrap().forced_stop }

    pub fn active_ids(&self) -> Vec<String> {
        self.runtimes.lock().unwrap().iter().filter(|(_, r)| r.lock().unwrap().is_active()).map(|(k, _)| k.clone()).collect()
    }

    /// Agrega un aviso de 939 Host a la consola del servidor.
    pub fn note(&self, id: &str, msg: &str) {
        let line = format!("── 939 Host: {msg} ──");
        self.runtime(id).lock().unwrap().push_log(line.clone());
        self.events.console_line(id, &line);
    }

    pub fn console(&self, id: &str) -> Vec<String> {
        self.runtime(id).lock().unwrap().log.iter().cloned().collect()
    }

    pub fn send_command(&self, id: &str, cmd: &str) -> AppResult<()> {
        let rt = self.runtime(id);
        let mut rt = rt.lock().unwrap();
        if rt.status() != RunStatus::Running && rt.status() != RunStatus::Starting {
            return Err(AppError::invalid("El servidor no está encendido."));
        }
        let cmd = cmd.trim().trim_start_matches('/');
        if cmd.is_empty() { return Ok(()); }
        rt.write_stdin(cmd)?;
        rt.push_log(format!("> {cmd}"));
        Ok(())
    }

    /// Envía un comando interno y espera la primera línea de respuesta que contenga alguna de
    /// `needles`. Ni el comando ni la respuesta aparecen en la consola de 939 Host.
    pub fn query(&self, id: &str, cmd: &str, needles: &[&str], timeout: Duration) -> AppResult<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let rt = self.runtime(id);
            let mut rt = rt.lock().unwrap();
            if rt.status() != RunStatus::Running {
                return Err(AppError::invalid("El servidor no está funcionando."));
            }
            rt.queries.retain(|q| q.until > Instant::now());
            rt.queries.push(Query { needles: needles.iter().map(|n| n.to_string()).collect(), tx, until: Instant::now() + timeout });
            rt.write_stdin(cmd)?;
        }
        rx.recv_timeout(timeout).map_err(|_| AppError::new("timeout", "El servidor no respondió a tiempo."))
    }

    /// Pausa el mundo con el comando oficial `/tick freeze` (Minecraft 1.20.3 o posterior).
    pub fn pause(&self, cfg: &ServerConfig) -> AppResult<()> {
        if !supports_tick(&cfg.mc_version) {
            return Err(AppError::invalid("Pausar necesita Minecraft 1.20.3 o posterior."));
        }
        let line = self.query(&cfg.id, "tick freeze", &["The game is frozen", "Unknown or incomplete command"], Duration::from_secs(5))?;
        if line.contains("Unknown") { return Err(AppError::invalid("Esta versión del servidor no permite pausar.")); }
        self.runtime(&cfg.id).lock().unwrap().paused = true;
        self.note(&cfg.id, "mundo en pausa (/tick freeze)");
        self.changed(&cfg.id);
        Ok(())
    }

    /// Reanuda el mundo pausado.
    pub fn resume(&self, id: &str) -> AppResult<()> {
        self.query(id, "tick unfreeze", &["The game is running normally", "The game is not frozen", "Unknown or incomplete command"], Duration::from_secs(5))?;
        self.runtime(id).lock().unwrap().paused = false;
        self.note(id, "mundo reanudado");
        self.changed(id);
        Ok(())
    }

    /// Cuenta regresiva con aviso en el chat. Devuelve true si llegó a cero (false si se canceló
    /// o el servidor dejó de estar encendido). Bloquea: llamarla desde un hilo propio.
    pub fn countdown(&self, id: &str, seconds: u32, kind: &str) -> bool {
        let rt_arc = self.runtime(id);
        let gen = {
            let mut rt = rt_arc.lock().unwrap();
            if rt.status() != RunStatus::Running { return false; }
            rt.countdown_gen += 1;
            rt.countdown = Some(Countdown {
                kind: kind.into(),
                ends_at: (chrono::Local::now() + chrono::Duration::seconds(seconds as i64)).to_rfc3339(),
                seconds,
            });
            rt.countdown_gen
        };
        self.changed(id);
        let verb = if kind == "restart" { "se reinicia" } else { "se detiene" };
        let marks: Vec<u32> = [60, 30, 10, 5, 3, 2, 1].into_iter().filter(|m| *m <= seconds).collect();
        let start = Instant::now();
        let mut last_said: Option<u32> = None;
        loop {
            let left = seconds.saturating_sub(start.elapsed().as_secs() as u32);
            {
                let mut rt = rt_arc.lock().unwrap();
                if rt.countdown_gen != gen || rt.status() != RunStatus::Running { return false; }
                if left == 0 { rt.countdown = None; break; }
                let say = match last_said {
                    None => { let _ = rt.write_stdin("save-all"); Some(format!("say El servidor {verb} en {left} segundos. Se guarda el progreso de todos.")) }
                    Some(prev) if prev != left && marks.contains(&left) => Some(format!("say El servidor {verb} en {left}…")),
                    _ => None,
                };
                if let Some(msg) = say { let _ = rt.write_stdin(&msg); last_said = Some(left); }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        self.changed(id);
        true
    }

    /// Cancela una cuenta regresiva en curso.
    pub fn cancel_countdown(&self, id: &str) -> bool {
        let rt_arc = self.runtime(id);
        let mut rt = rt_arc.lock().unwrap();
        if rt.countdown.is_none() { return false; }
        rt.countdown = None;
        rt.countdown_gen += 1;
        let _ = rt.write_stdin("say Cancelado: el servidor sigue encendido.");
        drop(rt);
        self.changed(id);
        true
    }

    pub fn set_restarting(&self, id: &str, on: bool) {
        self.runtime(id).lock().unwrap().restarting = on;
        self.changed(id);
    }

    pub fn start(&self, probe: &Probe, cfg: &ServerConfig) -> AppResult<()> {
        if !cfg.installed {
            return Err(AppError::invalid("El servidor todavía no terminó de instalarse."));
        }
        if cfg.eula_accepted_at.is_none() {
            return Err(AppError::invalid("Falta aceptar el EULA de Minecraft en los ajustes del servidor."));
        }
        let rt_arc = self.runtime(&cfg.id);
        {
            let rt = rt_arc.lock().unwrap();
            if rt.is_active() { return Err(AppError::busy("El servidor ya está encendido o en proceso de cambio.")); }
            if let Some(b) = &rt.busy { return Err(AppError::busy(format!("Esperá a que termine: {b}."))); }
        }
        crate::lease::can_start(&cfg.id)?;
        // Mods recibidos de otras PCs sin aprobación de la propietaria: no se cargan.
        let (held, back) = crate::modguard::enforce(&cfg.id)?;
        if !held.is_empty() { self.note(&cfg.id, &format!("{} mod(s) recibidos de otra PC esperan la aprobación de la propietaria y no se cargan: {}", held.len(), held.join(", "))); }
        if !back.is_empty() { self.note(&cfg.id, &format!("Mods aprobados por la propietaria vuelven a cargarse: {}", back.join(", "))); }
        let run_dir = paths::server_run_dir(&cfg.id);
        // El lanzador de Fabric arranca siempre el servidor oficial (un archivo sincronizado no
        // puede cambiar qué jar se ejecuta).
        if cfg.loader == crate::model::Loader::Fabric && run_dir.join("fabric-server-launcher.properties").exists() {
            let _ = paths::write_atomic(&run_dir.join("fabric-server-launcher.properties"), b"serverJar=server.jar\n");
        }
        check_lock(probe, &run_dir)?;
        let resolved = self.resolve_port(cfg)?;
        let cfg = &resolved;
        // La configuración es la fuente de verdad: así el puerto y las opciones de esta PC valen
        // aunque server.properties venga de otra PC (traspaso o restauración).
        crate::properties::write_for(cfg)?;
        let java = java::installed_java(cfg.java_major)
            .ok_or_else(|| AppError::new("java", format!("Falta Java {}. Reinstalá el servidor desde Ajustes → Avanzado.", cfg.java_major)))?;
        let launch = launch_target(cfg)
            .ok_or_else(|| AppError::new("missing", "Falta el archivo del servidor. Reinstalalo desde Ajustes → Avanzado."))?;
        crate::log4shell::check(&cfg.mc_version, &run_dir)?;

        let mut cmd = Command::new(&java);
        java::sanitize_env(&mut cmd);
        cmd.current_dir(&run_dir).args(jvm_args(cfg));
        match &launch {
            Launch::Jar(j) => { cmd.arg("-jar").arg(j); }
            // Java 8 no entiende los archivos de argumentos («@archivo» llegó en Java 9): lo toma
            // como el nombre de la clase principal y no arranca («no se ha encontrado o cargado la
            // clase principal @…»). Con Java 8 (Minecraft 1.16 y anteriores) se pasan sueltos.
            Launch::ArgsFile(f) if cfg.java_major < 9 => {
                let text = std::fs::read_to_string(run_dir.join(f)).map_err(|e| AppError::new("missing", format!("No se pudo leer {f}: {e}. Reinstalá el servidor desde Ajustes → Avanzado.")))?;
                cmd.args(expand_args_file(&text));
            }
            Launch::ArgsFile(f) => { cmd.arg(format!("@{f}")); }
        }
        cmd.arg("nogui");
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
            cmd.creation_flags(CREATE_NO_WINDOW | ABOVE_NORMAL_PRIORITY_CLASS);
        }
        let mut child = cmd.spawn().map_err(|e| AppError::io("No se pudo iniciar Java", e))?;
        job::assign(&child);
        // Si 939 Host termina de golpe, el guardián detiene este Java con «stop» (guardando).
        if let Some(si) = child.stdin.as_ref() { crate::guardian::watch(&cfg.id, &child, si); }
        mark_running(&cfg.id);
        let pid = child.id();
        let _ = std::fs::write(run_dir.join(".939host.lock"), pid.to_string());

        {
            let mut rt = rt_arc.lock().unwrap();
            rt.status = Some(RunStatus::Starting);
            rt.pid = Some(pid);
            rt.stdin = child.stdin.take();
            rt.started_at = Some(chrono::Local::now());
            rt.players.clear();
            rt.last_error = None;
            rt.needs_restart = false;
            rt.stop_requested = false;
            rt.forced_stop = false;
            rt.ready_announced = false;
            rt.last_output = Some(Instant::now());
            rt.mspt = None;
            rt.mspt_p95 = None;
            rt.tps = None;
            rt.metrics_supported = crate::playeredit::release_at_least(&cfg.mc_version, 20, 3);
            rt.debug_metrics = None;
            rt.debug_running = false;
            rt.reset_live();
            rt.phase = Some("Abriendo Java".into());
            let mods = std::fs::read_dir(run_dir.join("mods")).map(|r| r.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".jar")).count()).unwrap_or(0);
            rt.join_ctx = (cfg.mc_version.clone(), cfg.loader.name().to_lowercase(), mods);
            rt.push_log(format!("── 939 Host: iniciando {} ({} {}) con Java {} ──", cfg.name, loader_name(cfg.loader), cfg.mc_version, cfg.java_major));
        }
        self.changed(&cfg.id);

        let port = cfg.port;
        for (stream, is_err) in [(child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), false), (child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>), true)] {
            let Some(stream) = stream else { continue };
            let rt = rt_arc.clone();
            let ev = self.events.clone();
            let id = cfg.id.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stream);
                for line in reader.split(b'\n').map_while(Result::ok) {
                    let line = String::from_utf8_lossy(&line).trim_end().to_string();
                    if line.is_empty() { continue; }
                    handle_line(&ev, &rt, &id, line, is_err, port);
                }
            });
        }

        // Espera de salida: decide si fue una detención pedida o una caída.
        {
            let rt = rt_arc.clone();
            let ev = self.events.clone();
            let id = cfg.id.clone();
            let run_dir = run_dir.clone();
            std::thread::spawn(move || {
                let status = child.wait();
                crate::guardian::release(&id);
                clear_running(&id);
                let _ = std::fs::remove_file(run_dir.join(".939host.lock"));
                let mut r = rt.lock().unwrap();
                r.stdin = None;
                r.pid = None;
                r.players.clear();
                r.open_sessions.close_all(&id, chrono::Local::now());
                r.reset_live();
                r.exited_unsaved = true;
                let code = status.ok().and_then(|s| s.code());
                if r.stop_requested || code == Some(0) {
                    r.status = Some(RunStatus::Stopped);
                    r.push_log("── 939 Host: servidor detenido ──".into());
                } else {
                    r.status = Some(RunStatus::Crashed);
                    let report = latest_crash_report(&run_dir);
                    let msg = r.last_error.clone().unwrap_or_else(|| match &report {
                        Some(name) => format!("El servidor se cerró inesperadamente. Hay un informe de error: {name}."),
                        None => format!("El servidor se cerró inesperadamente (código {}). Revisá la consola.", code.map(|c| c.to_string()).unwrap_or_else(|| "desconocido".into())),
                    });
                    r.last_error = Some(msg.clone());
                    r.push_log(format!("── 939 Host: {msg} ──"));
                }
                drop(r);
                ev.server_changed(&id);
            });
        }

        // Métricas: `tick query` cada 20 s mientras está encendido y la ventana está abierta (solo
        // las usa la interfaz; con 939 Host en la bandeja no se le pide trabajo extra a Minecraft).
        {
            // En un servidor compartido también con la ventana cerrada: las demás PCs del grupo
            // ven el tiempo por tick real de esta.
            let rt = rt_arc.clone();
            let sid = cfg.id.clone();
            let debug_dir = run_dir.join("debug");
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(20));
                let shared = crate::network::load_share(&sid).enabled;
                let mut r = rt.lock().unwrap();
                let wanted = shared || METRICS_WANTED.load(std::sync::atomic::Ordering::Relaxed);
                match r.status() {
                    RunStatus::Running if r.metrics_supported && wanted => {
                        if let Some(stdin) = r.stdin.as_mut() {
                            if writeln!(stdin, "tick query").and_then(|_| stdin.flush()).is_ok() {
                                r.suppress_until = Some(Instant::now() + Duration::from_secs(3));
                            }
                        }
                    }
                    // Sin `tick query`: una medición de 20 s con `debug start` / `debug stop` (el
                    // resultado dice cuántos ticks por segundo hizo el servidor). El archivo de
                    // perfil que deja Minecraft se borra enseguida.
                    RunStatus::Running if !r.metrics_supported && r.debug_metrics != Some(false) && (wanted || r.debug_running) => {
                        let cmd = if r.debug_running { "debug stop" } else { "debug start" };
                        if let Some(stdin) = r.stdin.as_mut() {
                            if writeln!(stdin, "{cmd}").and_then(|_| stdin.flush()).is_ok() {
                                r.debug_running = !r.debug_running;
                                r.suppress_until = Some(Instant::now() + Duration::from_secs(3));
                            }
                        }
                        if !r.debug_running {
                            let dir = debug_dir.clone();
                            std::thread::spawn(move || { std::thread::sleep(Duration::from_secs(4)); clean_debug_profiles(&dir); });
                        }
                    }
                    RunStatus::Starting | RunStatus::Running => {}
                    _ => break,
                }
                drop(r);
            });
        }
        Ok(())
    }

    /// Procesos de los servidores de 939 Host encendidos en esta PC: (id, nombre, pid).
    pub fn own_processes(&self) -> Vec<(String, String, u32)> {
        let rts: Vec<(String, Arc<Mutex<Runtime>>)> = self.runtimes.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        rts.into_iter().filter_map(|(id, rt)| {
            let pid = rt.lock().unwrap().pid?;
            Some((id.clone(), crate::instance::load(&id).map(|c| c.name).unwrap_or(id), pid))
        }).collect()
    }

    /// Antes de encender: si el puerto está ocupado, se averigua quién lo usa. Con el servidor
    /// escuchando solo en esta PC (sin acceso desde la red local) el número de puerto es interno
    /// —se juega por 939 Host, que lo lee de la configuración—, así que se pasa solo a uno libre.
    /// Con acceso desde la red local, alguien puede estar entrando por ese número: no se cambia
    /// sin preguntar (lo ofrece «Reparar»). Nunca se detiene ni se toca el otro programa.
    fn resolve_port(&self, cfg: &ServerConfig) -> AppResult<ServerConfig> {
        let mut cfg = cfg.clone();
        if crate::ports::is_free(cfg.port) { return Ok(cfg); }
        let occ = crate::ports::occupant(cfg.port, &self.own_processes());
        let others: Vec<u16> = crate::instance::list_configs().into_iter().filter(|c| c.id != cfg.id).map(|c| c.port).collect();
        let next = crate::ports::free_near(cfg.port, &others);
        if crate::access::lan_effective(&cfg.id) {
            return Err(AppError::new("port", format!(
                "El puerto {} lo usa {}. Tocá «Reparar» para pasar este servidor a {}. Como está activado el acceso desde la red local, quien entre directo por IP va a tener que usar el puerto nuevo.",
                cfg.port, occ.describe(), next.map(|p| p.to_string()).unwrap_or_else(|| "otro puerto libre".into()))));
        }
        let Some(next) = next else {
            return Err(AppError::new("port", format!("El puerto {} lo usa {} y no se encontró otro libre cerca. Cerrá ese programa o elegí un puerto en Ajustes → Red.", cfg.port, occ.describe())));
        };
        let before = cfg.port;
        cfg.port = next;
        crate::instance::save(&cfg)?;
        self.note(&cfg.id, &format!("El puerto {before} lo usa {}. Este servidor pasó al puerto {next}; con 939 Host tus amigos no tienen que cambiar nada.", occ.describe()));
        Ok(cfg)
    }

    /// Detención ordenada: `stop` por consola. Un servidor grande con mods puede tardar varios
    /// minutos en guardar: mientras siga escribiendo en la consola se lo espera (hasta 10 min).
    /// Solo se cierra forzadamente si pasan 90 s sin terminar y además lleva 60 s en silencio.
    pub fn stop(&self, id: &str) -> AppResult<()> {
        let rt_arc = self.runtime(id);
        let pid = {
            let mut rt = rt_arc.lock().unwrap();
            match rt.status() {
                RunStatus::Running | RunStatus::Starting => {}
                RunStatus::Stopping => return Ok(()),
                _ => return Err(AppError::invalid("El servidor no está encendido.")),
            }
            rt.stop_requested = true;
            rt.countdown = None;
            rt.countdown_gen += 1;
            rt.status = Some(RunStatus::Stopping);
            rt.push_log("── 939 Host: guardando y deteniendo… ──".into());
            if let Some(stdin) = rt.stdin.as_mut() {
                let _ = writeln!(stdin, "stop").and_then(|_| stdin.flush());
            }
            rt.pid
        };
        self.changed(id);
        let rt = rt_arc.clone();
        let ev = self.events.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let begun = Instant::now();
            loop {
                {
                    let r = rt.lock().unwrap();
                    if !r.is_active() { return; }
                    let quiet = r.last_output.map(|t| t.elapsed()).unwrap_or(begun.elapsed());
                    if stop_gives_up(begun.elapsed(), quiet) { break; }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if let Some(pid) = pid {
                {
                    let mut r = rt.lock().unwrap();
                    r.forced_stop = true;
                    r.push_log(format!("── 939 Host: el servidor dejó de responder ({} s sin terminar de guardar); se cerró forzadamente ──", begun.elapsed().as_secs()));
                }
                force_kill(pid);
                ev.server_changed(&id);
            }
        });
        Ok(())
    }

    pub fn kill(&self, id: &str) -> AppResult<()> {
        let rt = self.runtime(id);
        let pid = {
            let mut r = rt.lock().unwrap();
            r.stop_requested = true;
            r.pid
        };
        match pid {
            Some(pid) => { force_kill(pid); self.changed(id); Ok(()) }
            None => Err(AppError::invalid("El servidor no está encendido.")),
        }
    }

    /// ¿Terminó el proceso sin que después se haya tomado un punto? (lo consume: devuelve true una vez)
    pub fn take_exit_unsaved(&self, id: &str) -> bool {
        let rt = self.runtime(id);
        let mut r = rt.lock().unwrap();
        if r.is_active() { return false; }
        // También vale lo que dejó anotado el guardián tras un cierre inesperado de 939 Host.
        let marked = take_exit_mark(id);
        std::mem::take(&mut r.exited_unsaved) || marked
    }

    /// Vuelve a comprobar la conexión real de un servidor encendido.
    pub fn check_connection(&self, id: &str, port: u16) -> AppResult<()> {
        let rt = self.runtime(id);
        if rt.lock().unwrap().status() != RunStatus::Running { return Err(AppError::busy("El servidor no está funcionando.")); }
        spawn_reach_check(self.events.clone(), rt, id.to_string(), port);
        Ok(())
    }

    /// Olvida el último problema de entrada (después de que el usuario lo leyó).
    pub fn clear_join_issue(&self, id: &str) { self.runtime(id).lock().unwrap().join_issue = None; self.changed(id); }

    /// Espera a que el servidor quede detenido (o caído). Devuelve false si venció el tiempo.
    pub fn wait_inactive(&self, id: &str, timeout: Duration) -> bool {
        let rt = self.runtime(id);
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !rt.lock().unwrap().is_active() { return true; }
            std::thread::sleep(Duration::from_millis(300));
        }
        false
    }

    /// Detiene todos los servidores (al salir de la app). Cada uno tiene la misma espera adaptativa
    /// que `stop` (no se corta un guardado que avanza); acá solo se espera a que terminen.
    pub fn stop_all(&self) {
        let ids = self.active_ids();
        for id in &ids { let _ = self.stop(id); }
        for id in &ids { self.wait_inactive(id, Duration::from_secs(620)); }
    }

    /// Envía un comando y espera la confirmación de guardado de Minecraft ("Saved the game").
    pub fn command_and_wait_save(&self, id: &str, cmd: &str, timeout: Duration) -> AppResult<()> {
        let rt = self.runtime(id);
        let before = rt.lock().unwrap().save_counter;
        self.send_command(id, cmd)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if rt.lock().unwrap().save_counter > before { return Ok(()); }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(AppError::new("timeout", "El servidor no confirmó el guardado a tiempo."))
    }
}

fn handle_line(ev: &Arc<dyn Events>, rt: &Arc<Mutex<Runtime>>, id: &str, line: String, is_err: bool, port: u16) {
    rt.lock().unwrap().last_output = Some(Instant::now());
    let mut changed = false;
    let mut hide = false;
    let mut probe_after_start = false;
    {
        let mut r = rt.lock().unwrap();
        // Respuestas de consultas internas: se entregan y no se muestran.
        let now = Instant::now();
        r.queries.retain(|q| q.until > now);
        if let Some(i) = r.queries.iter().position(|q| q.needles.iter().any(|n| line.contains(n.as_str()))) {
            let q = r.queries.remove(i);
            let _ = q.tx.send(line.clone());
            hide = true;
        }
        if line.contains("The game is frozen") { if !r.paused { r.paused = true; changed = true; } }
        if line.contains("The game is running normally") || line.contains("The game is not frozen") { if r.paused { r.paused = false; changed = true; } }
        // Solo «Saving the game» (save-all) tiene un final seguro («Saved the game»). Desde 26.3,
        // Minecraft escribe «Saving chunks for level…» al terminar de arrancar sin ningún mensaje
        // de cierre: tomarlo como guardado dejaba el estado en «Guardando» para siempre.
        if line.contains("Saving the game") {
            if !r.saving { r.saving = true; changed = true; }
        }
        if line.contains("Saved the game") || line.contains("All dimensions are saved") || line.contains("All chunks are saved") {
            if r.saving { r.saving = false; changed = true; }
        }
        if r.status() == RunStatus::Starting {
            if let Some(p) = startup_phase(&line) { r.phase = Some(p); changed = true; }
        }
        if let Some(until) = r.suppress_until {
            if Instant::now() < until && TICK_PATTERNS.iter().any(|p| line.contains(p)) {
                hide = true;
                if line.contains("Unknown or incomplete command") {
                    if r.metrics_supported { r.metrics_supported = false; } else { r.debug_metrics = Some(false); r.debug_running = false; }
                }
            }
        }
        if let Some(c) = RE_DEBUG_TPS.captures(&line) {
            let tps: Option<f64> = c[3].replace(',', ".").parse().ok();
            r.tps = tps.map(|t| t.min(20.0));
            // Con menos de 20 TPS el servidor tarda más de 50 ms por tick: se puede estimar.
            r.mspt = tps.filter(|t| *t < 19.5 && *t > 0.0).map(|t| 1000.0 / t);
            r.debug_metrics = Some(true);
            r.debug_running = false;
            changed = true;
        }
        if let Some(c) = RE_AVG.captures(&line) { r.mspt = c[1].parse().ok(); r.tps = r.mspt.map(|m| (1000.0 / m.max(0.01)).min(20.0)); changed = true; }
        if let Some(c) = RE_P95.captures(&line) { r.mspt_p95 = c[1].parse().ok(); }
        if r.status() == RunStatus::Starting && RE_DONE.is_match(&line) {
            r.status = Some(RunStatus::Running);
            r.phase = None;
            r.restarting = false;
            // «Done» no alcanza: se comprueba de verdad que el puerto atiende a Minecraft.
            r.reach = Some("checking".into());
            probe_after_start = true;
            changed = true;
        }
        {
            let (v, l, m) = r.join_ctx.clone();
            if let Some(issue) = r.join_watch.line(&line, &v, &l, m) {
                r.push_log(format!("── 939 Host: {} — {} ──", issue.title, issue.detail));
                r.join_issue = Some(issue);
                changed = true;
            }
        }
        // Entradas y salidas: solo líneas del propio servidor, nunca texto del chat.
        match crate::sessions::parse(&line) {
            Some(crate::sessions::Event::Join { name, previously }) => {
                if let Some(old) = &previously { r.players.remove(old); }
                r.players.insert(name.clone());
                r.open_sessions.join(&name, previously, chrono::Local::now());
                changed = true;
            }
            Some(crate::sessions::Event::Leave { name }) => {
                r.players.remove(&name);
                r.open_sessions.leave(id, &name, chrono::Local::now());
                changed = true;
            }
            None => {}
        }
        if line.contains("Saved the game") { r.save_counter += 1; }
        if line.contains("OutOfMemoryError") {
            r.last_error = Some("Al servidor le faltó memoria (OutOfMemoryError).".into());
        }
        if RE_BIND.is_match(&line) {
            r.last_error = Some(format!("El puerto {port} no se pudo abrir: lo está usando otro programa. Tocá «Reparar»: elige un puerto libre y lo vuelve a intentar."));
        }
        if line.contains("You need to agree to the EULA") {
            r.last_error = Some("Minecraft pide aceptar el EULA. Aceptalo en los ajustes del servidor.".into());
        }
        if !hide {
            let text = if is_err { format!("[stderr] {line}") } else { line.clone() };
            r.push_log(text);
        }
    }
    if !hide {
        ev.console_line(id, &line);
    }
    if changed { ev.server_changed(id); }
    if probe_after_start { spawn_reach_check(ev.clone(), rt.clone(), id.to_string(), port); }
}

/// Comprueba en segundo plano que el servidor responda como Minecraft en 127.0.0.1:puerto.
pub fn spawn_reach_check(ev: Arc<dyn Events>, rt: Arc<Mutex<Runtime>>, id: String, port: u16) {
    std::thread::spawn(move || {
        { let mut r = rt.lock().unwrap(); r.reach = Some("checking".into()); r.reach_error = None; }
        ev.server_changed(&id);
        let res = crate::reach::status_retry_for("127.0.0.1", port, Duration::from_secs(60));
        let mut ready = false;
        {
            let mut r = rt.lock().unwrap();
            if r.status() != RunStatus::Running { return; }
            match res {
                Ok(info) => {
                    r.push_log(format!("── 939 Host: conexión comprobada · Minecraft {} (protocolo {}) responde en 127.0.0.1:{port} en {} ms ──", info.version, info.protocol, info.latency_ms));
                    r.reach = Some("ok".into()); r.reach_info = Some(info); r.reach_error = None;
                    ready = !std::mem::replace(&mut r.ready_announced, true);
                }
                Err(e) => {
                    r.push_log(format!("── 939 Host: el servidor dice estar listo pero no atiende conexiones en el puerto {port}: {e} ──"));
                    r.reach = Some("fail".into()); r.reach_error = Some(e);
                }
            }
        }
        ev.server_changed(&id);
        if ready { ev.server_ready(&id); }
    });
}

fn loader_name(l: Loader) -> &'static str { l.name() }

static RE_PCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{1,3})\s?%").unwrap());

/// Traduce las líneas del arranque a una etapa entendible.
fn startup_phase(line: &str) -> Option<String> {
    let l = line.to_lowercase();
    if l.contains("loading libraries") || l.contains("unbundling") || l.contains("loading minecraft") { return Some("Cargando Minecraft".into()); }
    if l.contains("loading") && l.contains("mods") { return Some("Cargando mods".into()); }
    if l.contains("loading properties") || l.contains("default game type") { return Some("Leyendo la configuración".into()); }
    if l.contains("preparing level") { return Some("Abriendo el mundo".into()); }
    if l.contains("preparing spawn area") || l.contains("preparing start region") {
        return Some(match RE_PCT.captures(line) { Some(c) => format!("Preparando el mundo · {} %", &c[1]), None => "Preparando el mundo".into() });
    }
    None
}

/// `/tick` existe desde Minecraft 1.20.3 (snapshot 23w43a). Las versiones con numeración nueva
/// (26.1, 26.2…) también lo tienen.
pub fn supports_tick(mc: &str) -> bool {
    let mc = mc.trim();
    if let Some(rest) = mc.strip_prefix("1.") {
        let nums: Vec<u32> = rest.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).take(2).map(|p| p.parse().unwrap_or(0)).collect();
        let (minor, patch) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0));
        return minor > 20 || (minor == 20 && patch >= 3);
    }
    if mc.len() >= 5 && mc.as_bytes()[2] == b'w' {
        let yy: u32 = mc[0..2].parse().unwrap_or(0);
        let ww: u32 = mc[3..5].parse().unwrap_or(0);
        return (yy, ww) >= (23, 43);
    }
    mc.split('.').next().and_then(|n| n.parse::<u32>().ok()).map(|n| n >= 2).unwrap_or(false)
}

/// Separa un archivo de argumentos de Java (`@archivo`) como lo hace el propio Java 9+: espacios o
/// saltos de línea separan, las comillas (simples o dobles) agrupan, `#` al inicio de línea es
/// un comentario.
pub fn expand_args_file(text: &str) -> Vec<String> {
    let mut out = vec![];
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('#') { continue; }
        let (mut cur, mut quote, mut has) = (String::new(), None::<char>, false);
        for ch in t.chars() {
            match (quote, ch) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => cur.push(c),
                (None, '"') | (None, '\'') => { quote = Some(ch); has = true; }
                (None, c) if c.is_whitespace() => { if has || !cur.is_empty() { out.push(std::mem::take(&mut cur)); has = false; } }
                (None, c) => cur.push(c),
            }
        }
        if has || !cur.is_empty() { out.push(cur); }
    }
    out
}

/// Cómo se arranca el servidor según el loader.
#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    /// `java … -jar <archivo> nogui`
    Jar(String),
    /// `java … @<archivo de argumentos> nogui` (Forge 1.17+ y NeoForge)
    ArgsFile(String),
}

/// Encuentra el punto de arranque instalado. Forge/NeoForge modernos dejan un archivo de
/// argumentos en libraries/…/win_args.txt (unix_args.txt fuera de Windows); Forge viejo deja un jar.
pub fn launch_target(cfg: &ServerConfig) -> Option<Launch> {
    let run = paths::server_run_dir(&cfg.id);
    let args_name = if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" };
    match cfg.loader {
        Loader::Vanilla => run.join("server.jar").exists().then(|| Launch::Jar("server.jar".into())),
        Loader::Fabric => {
            if run.join("fabric-server-launch.jar").exists() { Some(Launch::Jar("fabric-server-launch.jar".into())) }
            else if run.join("939host-fabric-args.txt").exists() { Some(Launch::ArgsFile("939host-fabric-args.txt".into())) }
            else { None }
        }
        Loader::Forge | Loader::NeoForge => {
            let v = cfg.loader_version.clone().unwrap_or_default();
            let rel = match cfg.loader {
                Loader::Forge => format!("libraries/net/minecraftforge/forge/{}-{}/{}", cfg.mc_version, v, args_name),
                _ => format!("libraries/net/neoforged/neoforge/{}/{}", v, args_name),
            };
            if run.join(&rel).exists() { return Some(Launch::ArgsFile(rel)); }
            // Forge anterior a 1.17: jar "forge-<mc>-<versión>.jar" (o "-universal").
            std::fs::read_dir(&run).ok()?.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .find(|n| n.starts_with("forge-") && n.ends_with(".jar") && !n.contains("installer"))
                .map(Launch::Jar)
        }
    }
}

/// Memoria inicial del perfil equilibrado: un cuarto del máximo (al menos 1 GB). El máximo es el
/// que eligió la persona; el heap crece cuando hace falta.
///
/// Antes era Xms = Xmx con `AlwaysPreTouch` (como en un servidor dedicado): Windows reservaba y
/// tocaba todo el máximo al arrancar. En la CI, un mundo vacío con 6 GB ocupaba 6,5 GB de memoria
/// privada desde el primer segundo, en una PC donde la misma persona también juega.
pub fn initial_heap_mb(max_mb: u32) -> u32 { (max_mb / 4).max(1024).min(max_mb) }

/// Argumentos de la JVM según el perfil.
/// - Equilibrado (por defecto): heap inicial chico que crece (ver `initial_heap_mb`).
/// - Baja latencia (ZGC, elegido a propósito): Xms = Xmx y memoria reservada desde el inicio.
pub fn jvm_args(cfg: &ServerConfig) -> Vec<String> {
    let xms = if cfg.jvm_profile == JvmProfile::Balanced { initial_heap_mb(cfg.memory_mb) } else { cfg.memory_mb };
    let mut args = vec![format!("-Xms{xms}M"), format!("-Xmx{}M", cfg.memory_mb)];
    match cfg.jvm_profile {
        JvmProfile::Balanced => {
            args.extend([
                "-XX:+UseG1GC", "-XX:+ParallelRefProcEnabled", "-XX:MaxGCPauseMillis=200", "-XX:+UnlockExperimentalVMOptions",
                "-XX:+DisableExplicitGC", "-XX:G1NewSizePercent=30", "-XX:G1MaxNewSizePercent=40",
                "-XX:G1HeapRegionSize=8M", "-XX:G1ReservePercent=20", "-XX:G1HeapWastePercent=5", "-XX:G1MixedGCCountTarget=4",
                "-XX:InitiatingHeapOccupancyPercent=15", "-XX:G1MixedGCLiveThresholdPercent=90", "-XX:G1RSetUpdatingPauseTimePercent=5",
                "-XX:SurvivorRatio=32", "-XX:+PerfDisableSharedMem", "-XX:MaxTenuringThreshold=1",
            ].map(String::from));
        }
        JvmProfile::LowLatency => {
            args.extend(["-XX:+UseZGC", "-XX:+AlwaysPreTouch", "-XX:+DisableExplicitGC", "-XX:+PerfDisableSharedMem"].map(String::from));
            if cfg.java_major >= 21 && cfg.java_major < 23 { args.push("-XX:+ZGenerational".into()); }
        }
        JvmProfile::Custom => {}
    }
    // Encabezados de objeto compactos: producto desde Java 25 (menos memoria por objeto).
    if cfg.java_major >= 25 && cfg.jvm_profile != JvmProfile::Custom { args.push("-XX:+UseCompactObjectHeaders".into()); }
    // Java 24+ avisa en consola cuando una librería usa acceso nativo (JNA); se permite explícitamente.
    if cfg.java_major >= 24 { args.push("--enable-native-access=ALL-UNNAMED".into()); }
    args.push("-Dfile.encoding=UTF-8".into());
    args.extend(split_args(&cfg.extra_jvm_args));
    // Al final, para que ningún argumento extra la pise (Java toma el último -D repetido).
    args.extend(crate::log4shell::jvm_flags(&cfg.mc_version));
    args
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => { if !cur.is_empty() { out.push(std::mem::take(&mut cur)); } }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() { out.push(cur); }
    out
}

/// ¿Se deja de esperar a que el servidor termine de guardar? Nunca antes de 90 s; después, solo si
/// lleva 60 s sin escribir nada. Tope absoluto: 10 minutos.
pub fn stop_gives_up(elapsed: Duration, quiet: Duration) -> bool {
    elapsed >= Duration::from_secs(600) || (elapsed >= Duration::from_secs(90) && quiet >= Duration::from_secs(60))
}

static METRICS_WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// La interfaz está abierta (true) o 939 Host quedó solo en la bandeja (false): sin ventana no se
/// piden métricas de rendimiento a los servidores.
pub fn set_metrics_wanted(on: bool) { METRICS_WANTED.store(on, std::sync::atomic::Ordering::Relaxed); }

fn exit_mark(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("pendiente-publicar") }

/// El servidor se detuvo sin que 939 Host publicara su estado final (lo usa el guardián): la
/// próxima vez que 939 Host lo vea, lo trata como una salida sin publicar.
pub fn mark_exit_unsaved(id: &str) { let _ = std::fs::write(exit_mark(id), b"1"); }

/// Lee y quita la marca que dejó el guardián (true si había una).
pub fn take_exit_mark(id: &str) -> bool { std::fs::remove_file(exit_mark(id)).is_ok() }

fn running_mark(id: &str) -> std::path::PathBuf { paths::server_dir(id).join("en-marcha") }

/// Java está corriendo para este servidor. La marca se quita cuando 939 Host ve terminar el
/// proceso o cuando el guardián confirma que se detuvo. Si al volver a abrir 939 Host sigue ahí,
/// el cierre no lo vio nadie (corte de luz, «Finalizar árbol de procesos», falla de Windows):
/// lo que hay en disco no está confirmado como guardado.
pub fn mark_running(id: &str) {
    STARTED_HERE.lock().unwrap().insert(id.to_string());
    let _ = std::fs::write(running_mark(id), b"1");
}
pub fn clear_running(id: &str) {
    STARTED_HERE.lock().unwrap().remove(id);
    let _ = std::fs::remove_file(running_mark(id));
}

/// Servidores que este proceso de 939 Host encendió y todavía no vio terminar: su marca es la de
/// ahora, no la de un cierre anterior.
static STARTED_HERE: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

/// ¿Quedó la marca de un servidor que ya no corre? (no la quita: ver `take_unconfirmed`).
pub fn unconfirmed_close(id: &str) -> bool {
    running_mark(id).exists() && !STARTED_HERE.lock().unwrap().contains(id) && !is_pid_alive_for(id)
}

/// Lee y quita la marca de un cierre que nadie vio (true si había una y el servidor no corre).
pub fn take_unconfirmed(id: &str) -> bool { unconfirmed_close(id) && std::fs::remove_file(running_mark(id)).is_ok() }

/// Hay un Java de este servidor todavía vivo (por ejemplo, 939 Host se cerró y el guardián lo está
/// guardando): entonces no es un cierre sin ver.
fn is_pid_alive_for(id: &str) -> bool { crate::guardian::recovering(id) }

fn check_lock(probe: &Probe, run_dir: &std::path::Path) -> AppResult<()> {
    let lock = run_dir.join(".939host.lock");
    if let Ok(text) = std::fs::read_to_string(&lock) {
        if let Ok(pid) = text.trim().parse::<u32>() {
            if crate::hardware::process_alive(probe, pid) {
                if run_dir.parent().and_then(|d| d.file_name()).map(|n| crate::guardian::recovering(&n.to_string_lossy())).unwrap_or(false) {
                    return Err(AppError::busy("939 Host se cerró de golpe y este servidor se está guardando y deteniendo. Esperá unos segundos y probá de nuevo."));
                }
                return Err(AppError::busy("Este mundo ya está siendo ejecutado por otro proceso en esta PC. Para protegerlo, no se inicia dos veces."));
            }
        }
        let _ = std::fs::remove_file(&lock);
    }
    Ok(())
}

fn latest_crash_report(run_dir: &std::path::Path) -> Option<String> {
    let dir = run_dir.join("crash-reports");
    let recent = std::time::SystemTime::now() - Duration::from_secs(600);
    std::fs::read_dir(dir).ok()?.flatten()
        .filter_map(|e| { let m = e.metadata().ok()?.modified().ok()?; (m > recent).then(|| (m, e.file_name().to_string_lossy().to_string())) })
        .max_by_key(|(m, _)| *m)
        .map(|(_, n)| n)
}

fn force_kill(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(crate::paths::windows_program("taskkill.exe")).args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

#[cfg(windows)]
mod job {
    use once_cell::sync::OnceCell;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;

    static JOB: OnceCell<usize> = OnceCell::new();

    fn job() -> Option<usize> {
        JOB.get_or_try_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() { return Err(()); }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(h, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
            // Endurecimiento parcial (NO es un aislamiento): el servidor de Minecraft no tiene
            // interfaz, así que se le quita el acceso al portapapeles, al escritorio, a los
            // ajustes del sistema y a apagar Windows. Los mods igual pueden leer y escribir
            // archivos del usuario y usar la red: ver SECURITY.md.
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { UIRestrictionsClass: JOB_OBJECT_UILIMIT_READCLIPBOARD | JOB_OBJECT_UILIMIT_WRITECLIPBOARD
                | JOB_OBJECT_UILIMIT_DESKTOP | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS | JOB_OBJECT_UILIMIT_EXITWINDOWS | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS | JOB_OBJECT_UILIMIT_HANDLES };
            SetInformationJobObject(h, JobObjectBasicUIRestrictions, &ui as *const _ as *const _, std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32);
            Ok(h as usize)
        }).ok().copied()
    }

    pub fn assign(child: &std::process::Child) {
        if let Some(h) = job() {
            unsafe { AssignProcessToJobObject(h as _, child.as_raw_handle() as _); }
        }
    }

    pub fn handle() -> Option<usize> { job() }
}

/// Job Object de los servidores (para que el guardián lo mantenga abierto).
#[cfg(windows)]
pub fn job_handle() -> Option<usize> { job::handle() }

#[cfg(not(windows))]
mod job {
    pub fn assign(_child: &std::process::Child) {}
}

#[cfg(test)]
mod launch_tests {
    use super::*;

    fn cfg(loader: Loader, version: Option<&str>) -> ServerConfig {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).unwrap();
        let mut c: ServerConfig = serde_json::from_value(serde_json::json!({
            "id": format!("l{}", hex::encode(b)), "name": "x", "description": "", "mcVersion": "1.21.1", "loader": "vanilla",
            "loaderVersion": null, "javaMajor": 21, "memoryMb": 1024, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565,
            "maxPlayers": 4, "motd": "", "onlineMode": true, "whitelist": false, "difficulty": "easy", "gamemode": "survival",
            "hardcore": false, "pvp": true, "levelSeed": "", "levelType": "minecraft:normal", "viewDistance": 8, "simulationDistance": 6,
            "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15, "restoreIntervalMin": 15,
            "installed": true, "installError": null, "eulaAcceptedAt": "x", "createdAt": "x",
            "identity": { "publicKey": "a", "accessCode": "X" }, "extraProperties": {}
        })).unwrap();
        c.loader = loader;
        c.loader_version = version.map(String::from);
        c
    }

    /// Un guardado largo que sigue avanzando no se corta; uno colgado sí.
    #[test]
    fn detener_espera_mientras_guarda() {
        let s = Duration::from_secs;
        assert!(!stop_gives_up(s(30), s(30)), "nunca antes de 90 s");
        assert!(!stop_gives_up(s(200), s(5)), "a los 200 s sigue escribiendo: se espera");
        assert!(stop_gives_up(s(95), s(61)), "a los 95 s y un minuto en silencio: se corta");
        assert!(stop_gives_up(s(600), s(0)), "tope absoluto de 10 minutos");
    }

    #[test]
    fn memoria_inicial_chica_y_maximo_intacto() {
        assert_eq!(initial_heap_mb(6144), 1536);
        assert_eq!(initial_heap_mb(2048), 1024);
        assert_eq!(initial_heap_mb(768), 768, "nunca más que el máximo");
        let mut c = cfg(Loader::Vanilla, None);
        c.memory_mb = 6144;
        let a = jvm_args(&c);
        assert!(a.contains(&"-Xms1536M".to_string()) && a.contains(&"-Xmx6144M".to_string()));
        assert!(!a.iter().any(|x| x == "-XX:+AlwaysPreTouch"), "el perfil equilibrado no reserva todo al arrancar");
    }

    #[test]
    fn tick_support_by_version() {
        assert!(!supports_tick("1.20.2"));
        assert!(supports_tick("1.20.3"));
        assert!(supports_tick("1.21.4"));
        assert!(!supports_tick("1.19.4"));
        assert!(supports_tick("26.2"));
        assert!(supports_tick("24w14a"));
        assert!(!supports_tick("23w40a"));
    }

    #[test]
    fn startup_phases_in_words() {
        assert_eq!(startup_phase("[Worker-Main-1/INFO]: Preparing spawn area: 64%").as_deref(), Some("Preparando el mundo · 64 %"));
        assert_eq!(startup_phase("[Server thread/INFO]: Preparing level \"world\"").as_deref(), Some("Abriendo el mundo"));
        assert_eq!(startup_phase("random line"), None);
    }

    #[test]
    fn finds_launch_target_per_loader() {
        let _ = paths::init(std::env::temp_dir().join("939host-launch-tests"));
        let args = if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" };
        let neo = cfg(Loader::NeoForge, Some("21.1.77"));
        let run = paths::server_run_dir(&neo.id);
        assert_eq!(launch_target(&neo), None);
        let p = run.join("libraries/net/neoforged/neoforge/21.1.77").join(args);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "-cp x").unwrap();
        assert_eq!(launch_target(&neo), Some(Launch::ArgsFile(format!("libraries/net/neoforged/neoforge/21.1.77/{args}"))));

        let forge = cfg(Loader::Forge, Some("52.0.1"));
        let run = paths::server_run_dir(&forge.id);
        let p = run.join("libraries/net/minecraftforge/forge/1.21.1-52.0.1").join(args);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "-cp x").unwrap();
        assert!(matches!(launch_target(&forge), Some(Launch::ArgsFile(_))));
        // Lo que deja 939 Host para Fabric (y lo que pasa a mano con Java 8).
        let a = expand_args_file("-Dfabric.gameJarPath=server.jar\n-cp\n\"libraries/a b.jar;libraries/c.jar\"\n# comentario\nnet.fabricmc.loader.impl.launch.knot.KnotServer\n");
        assert_eq!(a, vec!["-Dfabric.gameJarPath=server.jar", "-cp", "libraries/a b.jar;libraries/c.jar", "net.fabricmc.loader.impl.launch.knot.KnotServer"]);

        let old = cfg(Loader::Forge, Some("14.23.5.2860"));
        let run = paths::server_run_dir(&old.id);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("forge-1.12.2-14.23.5.2860-installer.jar"), "x").unwrap();
        assert_eq!(launch_target(&old), None, "el instalador no es el servidor");
        std::fs::write(run.join("forge-1.12.2-14.23.5.2860.jar"), "x").unwrap();
        assert_eq!(launch_target(&old), Some(Launch::Jar("forge-1.12.2-14.23.5.2860.jar".into())));

        let fab = cfg(Loader::Fabric, Some("0.17"));
        let run = paths::server_run_dir(&fab.id);
        std::fs::create_dir_all(&run).unwrap();
        std::fs::write(run.join("fabric-server-launch.jar"), "x").unwrap();
        assert_eq!(launch_target(&fab), Some(Launch::Jar("fabric-server-launch.jar".into())));
    }
}
