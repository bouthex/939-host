//! Vigía de una actualización: aplica la versión nueva y la vuelve atrás si no abre.
//!
//! La app, antes de cerrarse para actualizar, copia sus propios archivos a
//! `actualizacion\anterior\`, deja el estado en `actualizacion\estado.json` y abre una copia de sí
//! misma (`actualizacion\939Host-vigia.exe --vigilar-actualizacion …`). Esa copia, sin ventana:
//!
//! 1. Espera a que 939 Host termine de cerrarse (guardando y pasando los servidores como siempre).
//! 2. Aplica la versión nueva: el instalador verificado (`/S /ESPERAR`) o, en la portable, los
//!    archivos del .zip verificado escritos al lado y reemplazados uno por uno.
//! 3. Abre la versión nueva y espera a que su interfaz confirme que arrancó bien.
//! 4. Si algo falla (instalador con error, archivos incompletos, la versión nueva no abre o se cierra
//!    sola), restaura los archivos anteriores, vuelve a abrir la versión anterior y deja anotado qué
//!    pasó para mostrarlo y poder reintentar.
//!
//! Nunca toca la carpeta de datos (servidores, mundos, identidad, configuración): solo la carpeta del
//! programa. Lleva el nombre `939Host-vigia.exe` para que el instalador (que espera a que no haya
//! ningún `939Host.exe`) no lo confunda con la app abierta.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Estado de una actualización en curso o recién terminada.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub from: String,
    pub to: String,
    /// "instalada" | "portable"
    pub mode: String,
    /// Carpeta del programa (donde está 939Host.exe).
    pub app_dir: PathBuf,
    /// Paquete ya verificado (instalador .exe o portable .zip).
    pub package: PathBuf,
    /// Archivos del programa que se copiaron a `anterior` (rutas relativas).
    #[serde(default)]
    pub files: Vec<String>,
    /// "preparada" | "instalando" | "abriendo" | "ok" | "revertida" | "fallida"
    pub phase: String,
    #[serde(default)]
    pub message: String,
    pub at: String,
    /// La interfaz ya mostró el resultado.
    #[serde(default)]
    pub shown: bool,
}

pub fn dir(data_root: &Path) -> PathBuf { data_root.join("actualizacion") }
pub fn state_path(data_root: &Path) -> PathBuf { dir(data_root).join("estado.json") }
pub fn backup_dir(data_root: &Path) -> PathBuf { dir(data_root).join("anterior") }

pub fn read(p: &Path) -> Option<Pending> { std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()) }

pub fn write(p: &Path, s: &Pending) -> std::io::Result<()> {
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).unwrap())?;
    std::fs::rename(&tmp, p)
}

/// Ejecutable de la app dentro de su carpeta.
pub const APP_EXE: &str = if cfg!(windows) { "939Host.exe" } else { "939host" };

/// Cuánto espera a que la versión nueva confirme que abrió.
const CONFIRM_WAIT: Duration = Duration::from_secs(150);
/// Cuánto espera a que la app termine de cerrarse (puede estar pasando servidores a otra PC).
const QUIT_WAIT: Duration = Duration::from_secs(240);

struct Log(PathBuf);
impl Log {
    fn line(&self, m: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.0) {
            let _ = writeln!(f, "{} {m}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    }
}

/// Entrada del vigía: `--vigilar-actualizacion <estado.json> <pid de la app>`. No vuelve.
pub fn run(state: PathBuf, pid: u32) -> ! {
    let log = Log(state.with_file_name("registro.txt"));
    let code = match supervise(&state, pid, &log) { Ok(()) => 0, Err(()) => 1 };
    std::process::exit(code)
}

fn set_phase(state: &Path, phase: &str, message: &str, log: &Log) -> Result<Pending, ()> {
    let mut s = read(state).ok_or(())?;
    s.phase = phase.into();
    s.message = message.into();
    s.at = chrono::Local::now().to_rfc3339();
    if phase == "revertida" || phase == "fallida" || phase == "ok" { s.shown = false; }
    log.line(&format!("[{phase}] {message}"));
    write(state, &s).map_err(|e| log.line(&format!("no se pudo escribir el estado: {e}")))?;
    Ok(s)
}

fn supervise(state: &Path, pid: u32, log: &Log) -> Result<(), ()> {
    let s = read(state).ok_or_else(|| log.line("no hay estado de actualización"))?;
    log.line(&format!("vigía: {} → {} ({})", s.from, s.to, s.mode));
    // 1) Esperar a que la app termine de cerrarse.
    if !wait_exit(pid, QUIT_WAIT) {
        set_phase(state, "fallida", "939 Host no terminó de cerrarse a tiempo. No se cambió nada; podés volver a intentarlo.", log)?;
        return Err(());
    }
    std::thread::sleep(Duration::from_millis(800));
    // 2) Aplicar.
    let s = set_phase(state, "instalando", "", log)?;
    let backup = state.with_file_name("anterior");
    if let Err(why) = apply(&s, log) {
        return revert(state, &s, &backup, &format!("No se pudo instalar 939 Host {}: {why}", s.to), log);
    }
    // 3) Abrir la versión nueva y esperar su confirmación.
    let s = set_phase(state, "abriendo", "", log)?;
    let mut child = match launch(&s.app_dir, "--tras-actualizar", true) {
        Ok(c) => c,
        Err(e) => return revert(state, &s, &backup, &format!("La versión {} no se pudo abrir ({e}).", s.to), log),
    };
    let end = Instant::now() + CONFIRM_WAIT;
    loop {
        if read(state).map(|p| p.phase == "ok").unwrap_or(false) { log.line("la versión nueva confirmó que abrió bien"); return Ok(()); }
        if let Ok(Some(st)) = child.try_wait() {
            // Se cerró sin confirmar: puede haber confirmado justo antes de salir.
            if read(state).map(|p| p.phase == "ok").unwrap_or(false) { return Ok(()); }
            return revert(state, &s, &backup, &format!("La versión {} se cerró al abrir ({st}).", s.to), log);
        }
        if Instant::now() > end {
            kill_tree(child.id());
            let _ = child.wait();
            return revert(state, &s, &backup, &format!("La versión {} no terminó de abrir en {} segundos.", s.to, CONFIRM_WAIT.as_secs()), log);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Instala el paquete verificado en la carpeta del programa.
fn apply(s: &Pending, log: &Log) -> Result<(), String> {
    if s.mode == "portable" {
        apply_zip(&s.package, &s.app_dir).map_err(|e| format!("no se pudieron escribir los archivos nuevos ({e})"))?;
        log.line("portable: archivos nuevos escritos");
        return Ok(());
    }
    let mut cmd = std::process::Command::new(&s.package);
    // /D= tiene que ir al final y sin comillas (regla de NSIS): misma carpeta que la instalación actual.
    cmd.args(["/S", "/ESPERAR"]);
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; cmd.raw_arg(format!("/D={}", s.app_dir.display())); }
    let st = cmd.status().map_err(|e| format!("no se pudo abrir el instalador ({e})"))?;
    log.line(&format!("instalador terminó: {st}"));
    if !st.success() { return Err(format!("el instalador terminó con error ({st})")); }
    if !s.app_dir.join(APP_EXE).is_file() { return Err("el instalador no dejó el programa".into()); }
    Ok(())
}

/// Escribe los archivos del .zip junto a los actuales (`.nuevo`) y recién cuando están todos
/// completos los pone en su lugar. Así un error a mitad no deja una mezcla.
pub fn apply_zip(zip_path: &Path, app_dir: &Path) -> Result<Vec<String>, String> {
    let f = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut staged = vec![];
    let res = (|| {
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            if e.is_dir() { continue; }
            let Some(rel) = e.enclosed_name() else { return Err(format!("ruta no permitida en el paquete: {}", e.name().unwrap_or_default())) };
            let dest = app_dir.join(&rel);
            if let Some(d) = dest.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
            let tmp = PathBuf::from(format!("{}.nuevo", dest.display()));
            let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut out).map_err(|e| e.to_string())?;
            out.sync_all().map_err(|e| e.to_string())?;
            staged.push((tmp, dest, rel.to_string_lossy().replace('\\', "/")));
        }
        Ok(())
    })();
    if let Err(e) = res {
        for (tmp, _, _) in &staged { let _ = std::fs::remove_file(tmp); }
        return Err(e);
    }
    let mut done = vec![];
    for (tmp, dest, rel) in staged {
        retry(|| std::fs::rename(&tmp, &dest)).map_err(|e| format!("{rel}: {e}"))?;
        done.push(rel);
    }
    Ok(done)
}

/// Archivos del programa que hay que respaldar antes de actualizar.
/// Instalada: todo lo de su carpeta (es solo del programa). Portable: lo que trae el .zip (la carpeta
/// puede ser Descargas u otra con archivos de la persona, que no se tocan).
pub fn files_to_backup(mode: &str, app_dir: &Path, package: &Path) -> Result<Vec<String>, String> {
    if mode == "portable" {
        let f = std::fs::File::open(package).map_err(|e| e.to_string())?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        let mut v = vec![];
        for i in 0..z.len() {
            let e = z.by_index(i).map_err(|e| e.to_string())?;
            if e.is_dir() { continue; }
            if let Some(n) = e.enclosed_name() { if app_dir.join(&n).is_file() { v.push(n.to_string_lossy().replace('\\', "/")); } }
        }
        return Ok(v);
    }
    let mut v = vec![];
    fn walk(base: &Path, d: &Path, v: &mut Vec<String>) -> std::io::Result<()> {
        for e in std::fs::read_dir(d)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() { walk(base, &p, v)?; } else if let Ok(r) = p.strip_prefix(base) { v.push(r.to_string_lossy().replace('\\', "/")); }
        }
        Ok(())
    }
    walk(app_dir, app_dir, &mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

/// Copia los archivos a `backup` y comprueba que quedaron iguales (tamaño y SHA-256).
pub fn backup(app_dir: &Path, files: &[String], backup: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(backup);
    for f in files {
        let (src, dst) = (app_dir.join(f), backup.join(f));
        if let Some(d) = dst.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        std::fs::copy(&src, &dst).map_err(|e| format!("{f}: {e}"))?;
        if sha_file(&src)? != sha_file(&dst)? { return Err(format!("la copia de {f} no quedó igual")); }
    }
    Ok(())
}

/// Vuelve a poner los archivos del respaldo en la carpeta del programa.
pub fn restore(app_dir: &Path, files: &[String], backup: &Path) -> Result<(), String> {
    for f in files {
        let (src, dst) = (backup.join(f), app_dir.join(f));
        if let Some(d) = dst.parent() { let _ = std::fs::create_dir_all(d); }
        retry(|| std::fs::copy(&src, &dst).map(|_| ())).map_err(|e| format!("{f}: {e}"))?;
    }
    for f in files { if sha_file(&backup.join(f))? != sha_file(&app_dir.join(f))? { return Err(format!("{f} no quedó como antes")); } }
    Ok(())
}

fn revert(state: &Path, s: &Pending, backup_dir: &Path, why: &str, log: &Log) -> Result<(), ()> {
    log.line(&format!("volviendo a la versión anterior: {why}"));
    match restore(&s.app_dir, &s.files, backup_dir) {
        Ok(()) => {
            if s.mode == "instalada" { restore_registry_version(&s.from); }
            set_phase(state, "revertida", &format!("{why} Se volvió a la versión {} y tus datos quedaron intactos. Podés reintentar desde Configuración → Actualizaciones.", s.from), log)?;
        }
        Err(e) => {
            set_phase(state, "fallida", &format!("{why} Tampoco se pudo restaurar la versión anterior ({e}): reinstalá 939 Host con el instalador; tus datos están intactos."), log)?;
            return Err(());
        }
    }
    let _ = launch(&s.app_dir, "--tras-revertir", false);
    Err(())
}

/// «Programas instalados» vuelve a mostrar la versión anterior.
fn restore_registry_version(v: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("reg").args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\939Host", "/v", "DisplayVersion", "/t", "REG_SZ", "/d", v, "/f"]).output();
    }
    let _ = v;
}

fn launch(app_dir: &Path, flag: &str, clean_env: bool) -> std::io::Result<std::process::Child> {
    let mut c = std::process::Command::new(app_dir.join(APP_EXE));
    c.arg(flag).current_dir(app_dir);
    // La versión fingida de las pruebas no pasa a la versión nueva.
    #[cfg(feature = "pruebas")]
    if clean_env { c.env_remove("H939_PRUEBA_VERSION_INSTALADA"); }
    let _ = clean_env;
    c.spawn()
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    { let _ = std::process::Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output(); }
    #[cfg(unix)]
    { let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).output(); }
}

fn wait_exit(pid: u32, max: Duration) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let end = Instant::now() + max;
    let mut sys = System::new();
    loop {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
        if sys.process(Pid::from_u32(pid)).is_none() { return true; }
        if Instant::now() > end { return false; }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// Un archivo recién liberado por un proceso que terminó puede seguir bloqueado unos instantes.
fn retry<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut last = None;
    for _ in 0..20 {
        match f() { Ok(v) => return Ok(v), Err(e) => { last = Some(e); std::thread::sleep(Duration::from_millis(500)); } }
    }
    Err(last.unwrap())
}

pub fn sha_file(p: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| e.to_string())?;
    Ok(hex::encode(h.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, b) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn portable_respaldo_aplicar_y_restaurar() {
        let t = tempfile::tempdir().unwrap();
        let app = t.path().join("app");
        std::fs::create_dir_all(app.join("licencias")).unwrap();
        std::fs::write(app.join(APP_EXE), b"viejo").unwrap();
        std::fs::write(app.join("licencias/a.txt"), b"lic vieja").unwrap();
        std::fs::write(app.join("mis-cosas.txt"), b"de la persona").unwrap();
        let pkg = t.path().join("p.zip");
        zip_with(&pkg, &[(APP_EXE, b"nuevo"), ("licencias/a.txt", b"lic nueva"), ("WebView2Loader.dll", b"dll")]);
        let files = files_to_backup("portable", &app, &pkg).unwrap();
        assert_eq!(files.len(), 2, "solo se respaldan los archivos del programa que existen: {files:?}");
        let bk = t.path().join("anterior");
        backup(&app, &files, &bk).unwrap();
        let done = apply_zip(&pkg, &app).unwrap();
        assert_eq!(done.len(), 3);
        assert_eq!(std::fs::read(app.join(APP_EXE)).unwrap(), b"nuevo");
        assert_eq!(std::fs::read(app.join("mis-cosas.txt")).unwrap(), b"de la persona");
        restore(&app, &files, &bk).unwrap();
        assert_eq!(std::fs::read(app.join(APP_EXE)).unwrap(), b"viejo");
        assert_eq!(std::fs::read(app.join("licencias/a.txt")).unwrap(), b"lic vieja");
        assert_eq!(std::fs::read(app.join("mis-cosas.txt")).unwrap(), b"de la persona");
    }

    #[test]
    fn paquete_roto_no_cambia_nada() {
        let t = tempfile::tempdir().unwrap();
        let app = t.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join(APP_EXE), b"viejo").unwrap();
        let pkg = t.path().join("p.zip");
        std::fs::write(&pkg, b"esto no es un zip").unwrap();
        assert!(apply_zip(&pkg, &app).is_err());
        assert_eq!(std::fs::read(app.join(APP_EXE)).unwrap(), b"viejo");
        // Rutas que escapan de la carpeta: se rechaza todo el paquete.
        zip_with(&pkg, &[(APP_EXE, b"nuevo"), ("../fuera.txt", b"x")]);
        assert!(apply_zip(&pkg, &app).is_err());
        assert_eq!(std::fs::read(app.join(APP_EXE)).unwrap(), b"viejo");
        assert!(!t.path().join("fuera.txt").exists());
        assert!(std::fs::read_dir(&app).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".nuevo")), "sin restos");
    }

    #[test]
    fn instalada_respalda_toda_su_carpeta() {
        let t = tempfile::tempdir().unwrap();
        let app = t.path().join("app");
        std::fs::create_dir_all(app.join("licencias")).unwrap();
        for f in [APP_EXE, "WebView2Loader.dll", "licencias/x.txt"] { std::fs::write(app.join(f), f.as_bytes()).unwrap(); }
        let mut files = files_to_backup("instalada", &app, Path::new("no-se-usa")).unwrap();
        files.sort();
        assert_eq!(files.len(), 3);
        let bk = t.path().join("anterior");
        backup(&app, &files, &bk).unwrap();
        std::fs::write(app.join(APP_EXE), b"roto").unwrap();
        std::fs::remove_file(app.join("licencias/x.txt")).unwrap();
        restore(&app, &files, &bk).unwrap();
        assert_eq!(std::fs::read(app.join(APP_EXE)).unwrap(), APP_EXE.as_bytes());
        assert!(app.join("licencias/x.txt").is_file());
    }
}
