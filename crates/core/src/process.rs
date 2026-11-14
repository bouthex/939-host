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
    pub started_at: Option<chrono::DateTime<chrono::Local>>,
    pub pid: Option<u32>,
    stdin: Option<ChildStdin>,
    pub log: VecDeque<String>,
    pub last_error: Option<String>,
    pub needs_restart: bool,
    stop_requested: bool,
    pub mspt: Option<f64>,
    pub mspt_p95: Option<f64>,
    metrics_supported: bool,
    suppress_until: Option<Instant>,
    pub save_counter: u64,
    pub installing: bool,
    pub busy: Option<String>,
}

impl Runtime {
    pub fn status(&self) -> RunStatus { self.status.unwrap_or(RunStatus::Stopped) }
    pub fn is_active(&self) -> bool { matches!(self.status(), RunStatus::Starting | RunStatus::Running | RunStatus::Stopping) }
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
static RE_JOIN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:\]|:) ([A-Za-z0-9_]{1,16}) joined the game").unwrap());
static RE_LEAVE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:\]|:) ([A-Za-z0-9_]{1,16}) left the game").unwrap());
static RE_AVG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)average time per tick:\s*([0-9.]+)\s*ms").unwrap());
static RE_P95: Lazy<Regex> = Lazy::new(|| Regex::new(r"P95:\s*([0-9.]+)\s*ms").unwrap());
static RE_BIND: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)failed to bind to port").unwrap());
const TICK_PATTERNS: &[&str] = &["The game is running normally", "Target tick rate", "Average time per tick", "Percentiles", "The game is frozen", "The game is sprinting", "Unknown or incomplete command", "<--[HERE]"];

impl Manager {
    pub fn new(events: Arc<dyn Events>) -> Self {
        Self { runtimes: Mutex::new(HashMap::new()), events }
    }

    pub fn changed(&self, id: &str) { self.events.server_changed(id); }

    pub fn runtime(&self, id: &str) -> Arc<Mutex<Runtime>> {
        self.runtimes.lock().unwrap().entry(id.to_string()).or_default().clone()
    }

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
        let stdin = rt.stdin.as_mut().ok_or_else(|| AppError::invalid("La consola del servidor no está disponible."))?;
        writeln!(stdin, "{cmd}").and_then(|_| stdin.flush()).map_err(|e| AppError::io("No se pudo enviar el comando", e))?;
        rt.push_log(format!("> {cmd}"));
        Ok(())
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
        let run_dir = paths::server_run_dir(&cfg.id);
        check_lock(probe, &run_dir)?;
        check_port(cfg.port)?;
        // La configuración es la fuente de verdad: así el puerto y las opciones de esta PC valen
        // aunque server.properties venga de otra PC (traspaso o restauración).
        crate::properties::write_for(cfg)?;
        let java = java::installed_java(cfg.java_major)
            .ok_or_else(|| AppError::new("java", format!("Falta Java {}. Reinstalá el servidor desde Ajustes → Avanzado.", cfg.java_major)))?;
        let launch = launch_target(cfg)
            .ok_or_else(|| AppError::new("missing", "Falta el archivo del servidor. Reinstalalo desde Ajustes → Avanzado."))?;

        let mut cmd = Command::new(&java);
        cmd.current_dir(&run_dir).args(jvm_args(cfg));
        match &launch {
            Launch::Jar(j) => { cmd.arg("-jar").arg(j); }
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
            rt.mspt = None;
            rt.mspt_p95 = None;
            rt.metrics_supported = true;
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
                    handle_line(&*ev, &rt, &id, line, is_err, port);
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
                let _ = std::fs::remove_file(run_dir.join(".939host.lock"));
                let mut r = rt.lock().unwrap();
                r.stdin = None;
                r.pid = None;
                r.players.clear();
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

        // Métricas: `tick query` cada 20 s mientras está encendido (salida oculta de la consola).
        {
            let rt = rt_arc.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(20));
                let mut r = rt.lock().unwrap();
                match r.status() {
                    RunStatus::Running if r.metrics_supported => {
                        if let Some(stdin) = r.stdin.as_mut() {
                            if writeln!(stdin, "tick query").and_then(|_| stdin.flush()).is_ok() {
                                r.suppress_until = Some(Instant::now() + Duration::from_secs(3));
                            }
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

    /// Detención ordenada: `stop` por consola y, si no responde en 90 s, cierre forzado.
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
            let deadline = Instant::now() + Duration::from_secs(90);
            while Instant::now() < deadline {
                if !rt.lock().unwrap().is_active() { return; }
                std::thread::sleep(Duration::from_millis(500));
            }
            if let Some(pid) = pid {
                rt.lock().unwrap().push_log("── 939 Host: el servidor no respondió en 90 s; se cerró forzadamente ──".into());
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

    /// Detiene todos los servidores (al salir de la app). Espera hasta 120 s en total.
    pub fn stop_all(&self) {
        let ids = self.active_ids();
        for id in &ids { let _ = self.stop(id); }
        let deadline = Instant::now() + Duration::from_secs(120);
        for id in &ids {
            let left = deadline.saturating_duration_since(Instant::now());
            if !self.wait_inactive(id, left) {
                if let Some(pid) = self.runtime(id).lock().unwrap().pid { force_kill(pid); }
            }
        }
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

fn handle_line(ev: &dyn Events, rt: &Arc<Mutex<Runtime>>, id: &str, line: String, is_err: bool, port: u16) {
    let mut changed = false;
    let mut hide = false;
    {
        let mut r = rt.lock().unwrap();
        if let Some(until) = r.suppress_until {
            if Instant::now() < until && TICK_PATTERNS.iter().any(|p| line.contains(p)) {
                hide = true;
                if line.contains("Unknown or incomplete command") { r.metrics_supported = false; }
            }
        }
        if let Some(c) = RE_AVG.captures(&line) { r.mspt = c[1].parse().ok(); changed = true; }
        if let Some(c) = RE_P95.captures(&line) { r.mspt_p95 = c[1].parse().ok(); }
        if r.status() == RunStatus::Starting && RE_DONE.is_match(&line) {
            r.status = Some(RunStatus::Running);
            changed = true;
        }
        if let Some(c) = RE_JOIN.captures(&line) { r.players.insert(c[1].to_string()); changed = true; }
        if let Some(c) = RE_LEAVE.captures(&line) { r.players.remove(&c[1]); changed = true; }
        if line.contains("Saved the game") { r.save_counter += 1; }
        if RE_BIND.is_match(&line) {
            r.last_error = Some(format!("El puerto {port} ya está en uso por otro programa. Cambiá el puerto en Ajustes → Red."));
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
}

fn loader_name(l: Loader) -> &'static str { l.name() }

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

/// Argumentos de la JVM según el perfil. Xms = Xmx para evitar que el heap crezca a saltos.
pub fn jvm_args(cfg: &ServerConfig) -> Vec<String> {
    let mut args = vec![format!("-Xms{}M", cfg.memory_mb), format!("-Xmx{}M", cfg.memory_mb)];
    match cfg.jvm_profile {
        JvmProfile::Balanced => {
            args.extend([
                "-XX:+UseG1GC", "-XX:+ParallelRefProcEnabled", "-XX:MaxGCPauseMillis=200", "-XX:+UnlockExperimentalVMOptions",
                "-XX:+DisableExplicitGC", "-XX:+AlwaysPreTouch", "-XX:G1NewSizePercent=30", "-XX:G1MaxNewSizePercent=40",
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

fn check_port(port: u16) -> AppResult<()> {
    // Se prueba en loopback: no dispara el aviso del firewall de Windows para 939 Host.
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => { drop(l); Ok(()) }
        Err(_) => Err(AppError::new("port", format!("El puerto {port} está ocupado (¿otro servidor o programa?). Cambialo en Ajustes → Red."))),
    }
}

fn check_lock(probe: &Probe, run_dir: &std::path::Path) -> AppResult<()> {
    let lock = run_dir.join(".939host.lock");
    if let Ok(text) = std::fs::read_to_string(&lock) {
        if let Ok(pid) = text.trim().parse::<u32>() {
            if crate::hardware::process_alive(probe, pid) {
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
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).status();
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
            Ok(h as usize)
        }).ok().copied()
    }

    pub fn assign(child: &std::process::Child) {
        if let Some(h) = job() {
            unsafe { AssignProcessToJobObject(h as _, child.as_raw_handle() as _); }
        }
    }
}

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
