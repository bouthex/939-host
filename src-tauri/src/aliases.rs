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
pub struct Aliases { local: Mutex<HashMap<String, (String, tokio::task::JoinHandle<()>)>>, shared: Mutex<std::collections::HashSet<String>>, errors: Mutex<HashMap<String, String>>,
    /// Direcciones que llegan del grupo (ver `GroupNode::group_alias`), por servidor.
    group: Mutex<HashMap<String, String>> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasInfo { pub id: String, pub label: String, pub address: String, pub ip: String, pub listening: bool, pub in_hosts: bool, pub error: Option<String> }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasStatus { pub items: Vec<AliasInfo>, pub hosts_pending: bool, pub hosts_readable: bool, pub suggestion: HashMap<String, String> }

/// Direcciones que esta PC tiene que atender: las elegidas acá y, para servidores de un grupo,
/// la que eligió la propietaria (viaja firmada con los datos públicos del servidor). Antes solo
/// existían en la PC que la eligió: en las demás PCs del grupo `nombre.939host` no llevaba a nada.
fn wanted(app: &AppHandle) -> Vec<(String, String)> {
    let ids: std::collections::HashSet<String> = instance::list_configs().into_iter().map(|c| c.id).collect();
    let mut out: Vec<(String, String)> = alias::load().into_iter().filter(|(id, _)| ids.contains(id)).collect();
    let group = app.state::<Aliases>().group.lock().unwrap().clone();
    for id in network::shared_servers() {
        if out.iter().any(|(i, _)| *i == id) { continue; }
        let Some(label) = group.get(&id).and_then(|l| alias::validate(l).ok()) else { continue };
        if out.iter().any(|(_, l)| *l == label) { continue; }
        out.push((id, label));
    }
    out
}

/// Abre o cierra las escuchas según las direcciones elegidas.
pub async fn sync(app: &AppHandle) {
    let st = app.state::<Aliases>();
    if let Some(n) = app.state::<Cluster>().try_node() {
        let g: HashMap<String, String> = network::shared_servers().into_iter().filter_map(|id| n.group_alias(&id).map(|a| (id, a))).collect();
        *st.group.lock().unwrap() = g;
    }
    let want = wanted(app);
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

fn entries(app: &AppHandle) -> Vec<Entry> { wanted(app).into_iter().map(|(id, label)| Entry { ip: alias::ip_for(&id), label }).collect() }

#[tauri::command]
pub async fn alias_status(app: AppHandle) -> AppResult<AliasStatus> {
    sync(&app).await;
    let hosts = std::fs::read(alias::hosts_path()).ok().map(|b| String::from_utf8_lossy(&b).to_string());
    let cur = hosts.as_deref().map(alias::current).unwrap_or_default();
    let want = entries(&app);
    let st = app.state::<Aliases>();
    let errs = st.errors.lock().unwrap().clone();
    let items = wanted(&app).into_iter().map(|(id, label)| {
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
    // En un grupo, la dirección viaja a las demás PCs (solo la propietaria firma ese dato).
    if let Ok(n) = app.state::<Cluster>().node().await { let _ = n.refresh_card(&id); }
    sync(&app).await;
    Ok(r)
}

/// Escribe el bloque en el archivo hosts. Windows muestra su aviso de permisos (UAC) para
/// «Windows PowerShell»; si se rechaza, no se cambia nada.
///
/// Lo que corre como administrador es el PowerShell de `System32` con un script fijo que vuelve a
/// validar cada entrada ([`alias::hosts_script`]), nunca `939Host.exe`: la carpeta de la app es
/// del usuario, y un programa elevado desde ahí cargaría DLL de esa carpeta (cualquier programa
/// del usuario podría dejar una ahí y conseguir permisos de administrador).
#[tauri::command]
pub async fn alias_apply_hosts(app: AppHandle) -> AppResult<()> { write_hosts_elevated(entries(&app)).await }

async fn write_hosts_elevated(list: Vec<Entry>) -> AppResult<()> {
    let script = alias::hosts_script(&list)?;
    let encoded = alias::powershell_encoded(&script);
    let ps = alias::powershell_path();
    let esc = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
    let launcher = format!("$p = Start-Process -FilePath '{}' -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{encoded}' -Verb RunAs -Wait -PassThru -WindowStyle Hidden; exit $p.ExitCode", esc(&ps));
    let out = tokio::task::spawn_blocking(move || {
        let mut c = std::process::Command::new(&ps);
        c.args(["-NoProfile", "-NonInteractive", "-Command", &launcher]);
        #[cfg(windows)]
        { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
        c.output()
    }).await.map_err(|e| AppError::new("hosts", "Se interrumpió").with_detail(e))?.map_err(|e| AppError::io("No se pudo pedir permiso a Windows", e))?;
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

#[tauri::command]
pub fn public_address_get(id: String) -> Option<String> { alias::public::get(&id) }

/// Guarda (o quita) la dirección pública opcional y la publica en la ficha del grupo.
#[tauri::command]
pub async fn public_address_set(app: AppHandle, id: String, address: Option<String>) -> AppResult<Option<String>> {
    instance::load(&id)?;
    alias::public::set(&id, address.as_deref())?;
    if let Ok(n) = app.state::<Cluster>().node().await { let _ = n.refresh_card(&id); }
    Ok(alias::public::get(&id))
}

/// Quita lo que 939 Host configuró en Windows fuera de su carpeta (para la versión portable o
/// antes de borrar la app a mano): inicio con Windows y las direcciones .939host del archivo hosts
/// (pide permiso de administrador para «Windows PowerShell»). No borra servidores ni mundos.
#[tauri::command]
pub async fn system_cleanup(app: AppHandle) -> AppResult<Vec<String>> {
    let mut done = vec![];
    let mut s = host939_core::settings::load();
    if s.start_with_windows { host939_core::settings::apply_autostart(false)?; s.start_with_windows = false; host939_core::settings::save(&s)?; }
    done.push("Inicio con Windows desactivado".to_string());
    alias::clear_all()?;
    sync(&app).await;
    let has_block = std::fs::read(alias::hosts_path()).map(|b| String::from_utf8_lossy(&b).contains("# >>> 939 Host")).unwrap_or(false);
    if has_block { write_hosts_elevated(vec![]).await?; done.push("Direcciones .939host quitadas del archivo hosts".into()); } else { done.push("El archivo hosts no tenía direcciones de 939 Host".into()); }
    Ok(done)
}

/* ───────── Conexión por IP para jugadores sin 939 Host ───────── */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntranceView { node: String, name: String, online: bool, is_me: bool, info: host939_cluster::EntranceInfo }

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalStatus {
    /// `Some(true)` permitido, `Some(false)` bloqueado, `None` sin decidir.
    allowed: Option<bool>,
    shared: bool,
    /// Esta PC puede cambiar el interruptor (es la propietaria o el servidor es solo de esta PC).
    can_change: bool,
    online_mode: bool,
    whitelist: bool,
    lan_access: bool,
    /// Esta PC acepta ser entrada pública.
    consent: bool,
    mapping: bool,
    my_port: u16,
    entrances: Vec<EntranceView>,
    /// Dirección para compartir: la de una entrada comprobada y en línea, si hay.
    best: Option<String>,
    best_verified: bool,
    /// Nombre propio (dominio o túnel) que guardó la propietaria, si hay.
    public_name: Option<String>,
    /// IP de esta PC en la red de la casa (para quien está en la misma red), si se pudo saber.
    lan_ip: Option<String>,
    game_port: u16,
}

#[tauri::command]
pub async fn external_status(app: AppHandle, id: String) -> AppResult<ExternalStatus> {
    let cfg = instance::load(&id)?;
    let share = network::load_share(&id);
    let st = host939_core::settings::load();
    let node = app.state::<Cluster>().node().await.ok().cloned();
    let me = node.as_ref().map(|n| n.id()).unwrap_or_default();
    let can_change = !share.enabled || share.member(&me).map(|m| m.role == "propietario").unwrap_or(false);
    let mut entrances: Vec<EntranceView> = node.as_ref().map(|n| n.entrances(&id)).unwrap_or_default().into_iter()
        .map(|(node, name, online, info)| EntranceView { is_me: node == me, node, name, online, info }).collect();
    // Primero las comprobadas y en línea; después las abiertas; las de esta PC desempatan.
    let rank = |e: &EntranceView| -> u8 { match (e.online, e.info.state.as_str(), e.info.addr.is_some()) { (true, "comprobada", true) => 0, (true, "abierta", true) => 1, (_, _, true) => 2, _ => 3 } };
    entrances.sort_by_key(|e| (rank(e), !e.is_me));
    let best = entrances.first().filter(|e| rank(e) <= 1).and_then(|e| e.info.addr.clone());
    let best_verified = entrances.first().map(|e| rank(e) == 0).unwrap_or(false);
    Ok(ExternalStatus {
        allowed: host939_core::access::external_allowed(&id), shared: share.enabled, can_change, online_mode: cfg.online_mode, whitelist: cfg.whitelist,
        lan_access: st.lan_access, consent: st.public_entrance, mapping: st.public_entrance_mapping, my_port: host939_core::access::entrance_port(&id),
        entrances, best, best_verified, public_name: alias::public::get(&id),
        lan_ip: host939_cluster::entrance::local_ipv4().map(|i| i.to_string()), game_port: cfg.port,
    })
}

/// Interruptor «Permitir conexiones sin 939 Host». Solo controla quién entra a jugar: no da
/// permisos de 939 Host ni para alojar. En servidores compartidos va firmado por la propietaria.
#[tauri::command]
pub async fn external_set(app: AppHandle, id: String, allow: bool) -> AppResult<()> {
    let mut cfg = instance::load(&id)?;
    let share = network::load_share(&id);
    let cl = app.state::<Cluster>();
    let who;
    if share.enabled {
        let n = cl.node().await?;
        n.set_external_access(&id, allow).map_err(|e| AppError::new("forbidden", e.to_string()))?;
        who = n.id();
    } else {
        if allow && !cfg.online_mode { return Err(AppError::invalid("Primero activá «Verificar cuentas con Mojang» (modo online): sin eso, cualquiera podría entrar con el nombre de un operador.")); }
        cfg.external_access = Some(allow);
        instance::save(&cfg)?;
        who = "esta PC".into();
    }
    host939_core::playeredit::audit(&id, if allow { "conexiones-sin-939host: permitidas" } else { "conexiones-sin-939host: bloqueadas" }, &who);
    // La red local cambia en server.properties (se aplica al reiniciar); la entrada, ya.
    let _ = host939_core::properties::write_for(&instance::load(&id)?);
    if let Ok(n) = cl.node().await { n.sync_entrances().await; }
    Ok(())
}

/// Esta PC acepta (o deja de aceptar) ser entrada pública desde Internet.
#[tauri::command]
pub async fn entrance_consent(app: AppHandle, consent: bool, mapping: Option<bool>) -> AppResult<()> {
    let mut st = host939_core::settings::load();
    st.public_entrance = consent;
    if let Some(m) = mapping { st.public_entrance_mapping = m; }
    host939_core::settings::save(&st)?;
    if let Ok(n) = app.state::<Cluster>().node().await { n.sync_entrances().await; }
    Ok(())
}

#[tauri::command]
pub async fn entrance_port_set(app: AppHandle, id: String, port: u16) -> AppResult<()> {
    instance::load(&id)?;
    host939_core::access::set_entrance_port(&id, port)?;
    if let Ok(n) = app.state::<Cluster>().node().await { n.sync_entrances().await; }
    Ok(())
}
