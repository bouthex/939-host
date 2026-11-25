//! Operaciones largas en curso (copias, restauraciones, mods, mundos, traspasos…): qué se está
//! haciendo, en qué etapa, el progreso real cuando se puede medir y cómo terminó.
//!
//! La interfaz las muestra en un indicador discreto (evento `op`). Nunca se inventa un
//! porcentaje: `fraction` es `None` cuando no hay una medida real. Si una operación termina con
//! error, el mensaje queda visible hasta que la persona lo cierra.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpView {
    pub id: u64,
    /// Servidor al que pertenece (vacío si es general).
    pub server: String,
    pub title: String,
    pub stage: String,
    pub fraction: Option<f64>,
    /// running | done | error | cancelled
    pub state: String,
    pub error: Option<String>,
    /// Se puede cancelar sin dejar nada a medias.
    pub cancellable: bool,
}

#[derive(Default)]
pub struct Ops { list: Mutex<Vec<(OpView, Arc<AtomicBool>)>> }

/// Una operación en curso. Si se suelta sin `done`/`fail`, cuenta como terminada.
pub struct Op { app: AppHandle, view: Mutex<OpView>, cancel: Arc<AtomicBool>, finished: AtomicBool }

impl Ops {
    pub fn begin(&self, app: &AppHandle, server: &str, title: &str, stage: &str, cancellable: bool) -> Op {
        let view = OpView { id: NEXT.fetch_add(1, Ordering::Relaxed), server: server.into(), title: title.into(), stage: stage.into(), fraction: None, state: "running".into(), error: None, cancellable };
        let cancel = Arc::new(AtomicBool::new(false));
        self.list.lock().unwrap().push((view.clone(), cancel.clone()));
        let _ = app.emit("op", &view);
        Op { app: app.clone(), view: Mutex::new(view), cancel, finished: AtomicBool::new(false) }
    }
    pub fn running(&self) -> Vec<OpView> { self.list.lock().unwrap().iter().map(|(v, _)| v.clone()).collect() }
    pub fn cancel(&self, id: u64) -> bool {
        self.list.lock().unwrap().iter().find(|(v, _)| v.id == id && v.cancellable).map(|(_, c)| { c.store(true, Ordering::SeqCst); true }).unwrap_or(false)
    }
    fn update(&self, v: &OpView) {
        let mut l = self.list.lock().unwrap();
        if v.state == "running" { if let Some(x) = l.iter_mut().find(|(x, _)| x.id == v.id) { x.0 = v.clone(); } } else { l.retain(|(x, _)| x.id != v.id); }
    }
}

impl Op {
    fn emit(&self) {
        let v = self.view.lock().unwrap().clone();
        use tauri::Manager;
        self.app.state::<Ops>().update(&v);
        let _ = self.app.emit("op", &v);
    }
    /// Nueva etapa, con progreso real si se conoce (0.0 a 1.0).
    pub fn stage(&self, stage: &str, fraction: Option<f64>) {
        { let mut v = self.view.lock().unwrap(); v.stage = stage.into(); v.fraction = fraction.map(|f| f.clamp(0.0, 1.0)); }
        self.emit();
    }
    pub fn cancelled(&self) -> bool { self.cancel.load(Ordering::SeqCst) }
    pub fn done(&self, stage: &str) {
        if self.finished.swap(true, Ordering::SeqCst) { return; }
        { let mut v = self.view.lock().unwrap(); v.state = if self.cancelled() { "cancelled" } else { "done" }.into(); v.stage = stage.into(); v.fraction = None; }
        self.emit();
    }
    pub fn fail(&self, msg: &str) {
        if self.finished.swap(true, Ordering::SeqCst) { return; }
        { let mut v = self.view.lock().unwrap(); v.state = "error".into(); v.error = Some(msg.into()); }
        self.emit();
    }
    /// Termina según el resultado y lo devuelve tal cual.
    pub fn finish<T>(&self, r: Result<T, host939_core::error::AppError>, ok_stage: &str) -> Result<T, host939_core::error::AppError> {
        match &r { Ok(_) => self.done(ok_stage), Err(e) => self.fail(&e.message) }
        r
    }
}

impl Drop for Op {
    fn drop(&mut self) { if !self.finished.load(Ordering::SeqCst) { self.done("Terminado"); } }
}
