//! Direcciones `nombre.939host`: escucha en la IP propia de cada servidor y mantiene el bloque
//! de 939 Host en el archivo hosts (con permiso de administrador, pedido por Windows).

use crate::network::Cluster;
use host939_core::alias::{self, Entry};
use host939_core::error::{AppError, AppResult};
use host939_core::process::Manager;
use host939_core::{instance, network};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager as _};

#[derive(Default)]
pub struct Aliases { local: Mutex<HashMap<String, (String, tokio::task::JoinHandle<()>)>>, shared: Mutex<std::collections::HashSet<String>>, errors: Mutex<HashMap<String, String>> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasInfo { pub id: String, pub label: String, pub address: String, pub ip: String, pub listening: bool, pub in_hosts: bool, pub error: Option<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasStatus { pub items: Vec<AliasInfo>, pub hosts_pending: bool, pub hosts_readable: bool, pub suggestion: HashMap<String, String> }

fn wanted() -> Vec<(String, String)> {
    let ids: std::collections::HashSet<String> = instance::list_configs().into_iter().map(|c| c.id).collect();
    alias::load().into_iter().filter(|(id, _)| ids.contains(id)).collect()
}

/// Abre o cierra las escuchas según las direcciones elegidas.
pub async fn sync(app: &AppHandle) {
    let want = wanted();
    let st = app.state::<Aliases>();
    let shared: std::collections::HashSet<String> = network::shared_servers().into_iter().collect();
    // Cerrar las que ya no van.
    {
        let mut l = st.local.lock().unwrap();
        l.retain(|id, (label, h)| { let keep = want.iter().any(|(i, lb)| i == id && lb == label) && !shared.contains(id); if !keep { h.abort(); } keep });
    }
    let cl = app.state::<Cluster>();
    let node = cl.node().await.ok();
    for (id, label) in &want {
        let ip = alias::ip_for(id);
        let res: Result<(), String> = if shared.contains(id) {
            if st.shared.lock().unwrap().contains(id) { continue; }
            match node {
                Some(n) => n.open_alias(id, ip).await.map(|_| { st.shared.lock().unwrap().insert(id.clone()); }).map_err(|e| e.to_string()),
                None => Err("la red del grupo no está lista".into()),
            }
        } else {
            if st.local.lock().unwrap().contains_key(id) { continue; }
            let app2 = app.clone();
            let id2 = id.clone();
            match alias::forward(ip, move || {
                let mgr = app2.state::<Manager>();
                let active = mgr.runtime(&id2).lock().unwrap().is_active();
                if active { instance::load(&id2).ok().map(|c| c.port) } else { None }
            }).await {
                Ok(h) => { st.local.lock().unwrap().insert(id.clone(), (label.clone(), h)); Ok(()) }
                Err(e) => Err(e.to_string()),
            }
        };
        let mut errs = st.errors.lock().unwrap();
        match res { Ok(()) => { errs.remove(id); } Err(e) => { errs.insert(id.clone(), e); } }
    }
    if let Some(n) = node {
        for id in &shared { if !want.iter().any(|(i, _)| i == id) { n.close_alias(id).await; st.shared.lock().unwrap().remove(id); } }
    }
}

fn entries() -> Vec<Entry> { wanted().into_iter().map(|(id, label)| Entry { ip: alias::ip_for(&id), label }).collect() }

#[tauri::command]
pub async fn alias_status(app: AppHandle) -> AppResult<AliasStatus> {
    sync(&app).await;
    let hosts = std::fs::read(alias::hosts_path()).ok().map(|b| String::from_utf8_lossy(&b).to_string());
    let cur = hosts.as_deref().map(alias::current).unwrap_or_default();
    let want = entries();
    let st = app.state::<Aliases>();
    let errs = st.errors.lock().unwrap().clone();
    let items = wanted().into_iter().map(|(id, label)| {
        let ip = alias::ip_for(&id);
        AliasInfo { in_hosts: cur.iter().any(|e| e.label == label && e.ip == ip), listening: !errs.contains_key(&id), error: errs.get(&id).cloned(), address: format!("{label}{}", alias::SUFFIX), ip: ip.to_string(), id, label }
    }).collect();
    let mut a = cur.clone(); a.sort_by(|x, y| x.label.cmp(&y.label));
    let mut b = want.clone(); b.sort_by(|x, y| x.label.cmp(&y.label));
    let suggestion = instance::list_configs().into_iter().map(|c| (c.id, alias::suggest(&c.name))).collect();
    Ok(AliasStatus { items, hosts_pending: a != b, hosts_readable: hosts.is_some(), suggestion })
}

#[tauri::command]
pub async fn alias_set(app: AppHandle, id: String, label: Option<String>) -> AppResult<Option<String>> {
    instance::load(&id)?;
    let r = alias::set(&id, label.as_deref().filter(|l| !l.trim().is_empty()))?;
    sync(&app).await;
    Ok(r)
}

/// Escribe el bloque en el archivo hosts. Windows muestra su aviso de permisos (UAC); si se
/// rechaza, no se cambia nada.
#[tauri::command]
pub async fn alias_apply_hosts() -> AppResult<()> {
    let list = entries();
    alias::check_entries(&list)?;
    let req = host939_core::paths::cache().join("pedido-hosts.txt");
    host939_core::paths::write_atomic(&req, alias::encode(&list).as_bytes())?;
    let exe = std::env::current_exe().map_err(|e| AppError::io("No se encontró 939 Host", e))?;
    let esc = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
    let script = format!("$p = Start-Process -FilePath '{}' -ArgumentList '--939-hosts','\"{}\"' -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode", esc(&exe), esc(&req));
    let out = tokio::task::spawn_blocking(move || {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        c.output()
    }).await.map_err(|e| AppError::new("hosts", "Se interrumpió").with_detail(e))?.map_err(|e| AppError::io("No se pudo pedir permiso a Windows", e))?;
    let _ = std::fs::remove_file(&req);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("canceled") || err.contains("cancelad") || err.contains("1223") { return Err(AppError::new("hosts", "Cancelaste el permiso de Windows: no se cambió nada.")); }
        return Err(AppError::new("hosts", "No se pudo actualizar el archivo hosts.").with_detail(err.lines().take(3).collect::<Vec<_>>().join(" ")));
    }
    // Verificación: lo que quedó escrito es lo pedido.
    let now = std::fs::read(alias::hosts_path()).map(|b| alias::current(&String::from_utf8_lossy(&b))).unwrap_or_default();
    if list.iter().any(|e| !now.contains(e)) { return Err(AppError::new("hosts", "Windows no guardó el cambio en el archivo hosts. Puede que un antivirus lo esté bloqueando.")); }
    Ok(())
}

/// Modo ayudante (se ejecuta con permisos de administrador): `939host.exe --939-hosts <pedido>`.
/// Solo acepta entradas `127.93.*` con nombres `.939host` y solo toca el bloque de 939 Host.
pub fn helper_main(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a == "--939-hosts")?;
    let code = (|| -> AppResult<()> {
        let path = args.get(i + 1).ok_or_else(|| AppError::invalid("Falta el pedido"))?;
        let txt = std::fs::read_to_string(path.trim_matches('"')).map_err(|e| AppError::io("No se pudo leer el pedido", e))?;
        let list = alias::decode(&txt)?;
        alias::write_hosts(&list)
    })();
    Some(if code.is_ok() { 0 } else { 2 })
}
