//! Entrada pública: jugadores **sin 939 Host** entran desde Internet con una dirección IP común.
//!
//! Cómo funciona:
//! - Una PC del grupo que lo acepta (Configuración → «Recibir jugadores de Internet en esta PC»)
//!   escucha en su IP local, en un puerto propio por servidor, y le pide al router que lo abra
//!   (UPnP; si no hay, NAT-PMP o PCP). Nada de servicios externos ni cuentas.
//! - Cada conexión que llega se lleva a la PC que aloja el servidor **en ese momento** por el
//!   mismo camino que usan los jugadores con 939 Host (`route_one`: directo o por el túnel
//!   cifrado del grupo, con relays si hace falta). Por eso la dirección no cambia cuando cambia la
//!   anfitriona, y una anfitriona detrás de CGNAT igual recibe jugadores.
//! - Solo se reenvía al puerto de Minecraft de ese servidor. No hay ningún camino desde la
//!   entrada hacia la administración de 939 Host, que exige identidad P2P firmada.
//! - Cada PC del grupo que sí está online prueba la entrada desde afuera (un ping de estado de
//!   Minecraft, como la lista de servidores del juego) y así se sabe si funciona de verdad.
//!
//! Límites honestos: si el proveedor de Internet usa CGNAT, o el router no abre puertos y nadie
//! los abre a mano, desde Internet no se llega a esa PC. Otra PC del grupo puede ser la entrada.

use super::*;
use host939_core::access;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntranceInfo {
    /// preparando | abierta | comprobada | sin-acceso | cgnat | error
    pub state: String,
    pub detail: String,
    /// Dirección para compartir: «ip:puerto» (IPv4 pública).
    pub addr: Option<String>,
    /// Dirección IPv6 global, si la PC tiene una (sin NAT, depende del firewall del router).
    #[serde(default)]
    pub addr_v6: Option<String>,
    /// IP local y puerto (para abrir el puerto a mano en el router).
    #[serde(default)]
    pub local: Option<String>,
    /// upnp | pcp | manual
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub checked_at: Option<String>,
    /// Chat de voz (Simple Voice Chat) para quien entra por esta dirección sin 939 Host: estado
    /// del puerto UDP de voz (abierto en el router, o qué abrir a mano).
    #[serde(default)]
    pub voice: Option<String>,
}

pub(crate) struct Open {
    port: u16,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    pub(crate) info: Arc<Mutex<EntranceInfo>>,
    _pm: Option<portmapper::Client>,
    upnp: Arc<tokio::sync::Mutex<Option<(igd_next::aio::Gateway<igd_next::aio::tokio::Tokio>, u16)>>>,
}

impl Drop for Open {
    fn drop(&mut self) {
        for t in &self.tasks { t.abort(); }
        // Cerrar el puerto en el router (lo mejor posible; vence solo en una hora).
        let upnp = self.upnp.clone();
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move { if let Some((gw, ext)) = upnp.lock().await.take() { let _ = gw.remove_port(igd_next::PortMappingProtocol::TCP, ext).await; } });
        }
    }
}

/// Límites por IP de jugadores externos: nadie puede ocupar la entrada con miles de conexiones.
#[derive(Default)]
pub(crate) struct Guard { active: HashMap<IpAddr, usize>, recent: HashMap<IpAddr, Vec<Instant>>, total: usize }
const PER_IP_ACTIVE: usize = 8;
const PER_IP_PER_MIN: usize = 40;
const TOTAL_ACTIVE: usize = 256;

impl Guard {
    fn enter(&mut self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let r = self.recent.entry(ip).or_default();
        r.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if self.recent.len() > 8192 { self.recent.retain(|_, v| !v.is_empty()); }
        let a = self.active.get(&ip).copied().unwrap_or(0);
        let r = self.recent.entry(ip).or_default();
        if a >= PER_IP_ACTIVE || r.len() >= PER_IP_PER_MIN || self.total >= TOTAL_ACTIVE { return false; }
        r.push(now);
        *self.active.entry(ip).or_default() += 1;
        self.total += 1;
        true
    }
    fn leave(&mut self, ip: IpAddr) {
        if let Some(a) = self.active.get_mut(&ip) { *a = a.saturating_sub(1); if *a == 0 { self.active.remove(&ip); } }
        self.total = self.total.saturating_sub(1);
    }
}

/// IP local de la interfaz que sale a Internet (sin mandar nada: `connect` en UDP solo elige ruta).
pub fn local_ipv4() -> Option<Ipv4Addr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    match s.local_addr().ok()?.ip() { IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip), _ => None }
}
fn global_ipv6() -> Option<std::net::Ipv6Addr> {
    let s = std::net::UdpSocket::bind("[::]:0").ok()?;
    s.connect("[2001:4860:4860::8888]:53").ok()?;
    match s.local_addr().ok()?.ip() { IpAddr::V6(ip) if access::is_global_v6(ip) => Some(ip), _ => None }
}

/// Solo pruebas: escuchar en esta IP, sin router, y permitir comprobaciones en loopback.
fn test_bind() -> Option<Ipv4Addr> {
    #[cfg(feature = "pruebas")]
    { return std::env::var("H939_ENTRANCE_BIND").ok().and_then(|v| v.parse().ok()); }
    #[allow(unreachable_code)]
    None
}

fn now() -> String { chrono::Local::now().to_rfc3339() }

impl GroupNode {
    /// Abre o cierra las entradas según el interruptor de cada servidor y la decisión de esta PC.
    /// Se llama en cada vuelta del grupo y cuando cambia algo (barato si no hay cambios).
    pub async fn sync_entrances(&self) {
        // Lo habitual: esta PC no es entrada y no tiene ninguna abierta. Sin leer nada más.
        if !host939_core::settings::load().public_entrance && self.inner.entrances.lock().unwrap().is_empty() { return; }
        let mut ids: std::collections::BTreeSet<String> = network::shared_servers().into_iter().collect();
        for c in host939_core::instance::list_configs() { ids.insert(c.id); }
        let want: Vec<(String, u16)> = ids.into_iter().filter(|id| access::internet_entrance(id)).map(|id| { let p = access::entrance_port(&id); (id, p) }).collect();
        let mut close = vec![];
        {
            let mut map = self.inner.entrances.lock().unwrap();
            map.retain(|id, o| { let keep = want.iter().any(|(w, p)| w == id && *p == o.port); if !keep { close.push(id.clone()); } keep });
        }
        drop(close);
        for (id, port) in want {
            if self.inner.entrances.lock().unwrap().contains_key(&id) { continue; }
            let open = self.open_entrance(&id, port).await;
            self.inner.entrances.lock().unwrap().insert(id, open);
        }
    }

    pub fn entrance_info(&self, server: &str) -> Option<EntranceInfo> { self.inner.entrance_info(server) }

    /// Entradas de todas las PCs del grupo para este servidor (la de esta PC y lo que informan las
    /// demás): (id de la PC, nombre, en línea, información).
    pub fn entrances(&self, server: &str) -> Vec<(String, String, bool, EntranceInfo)> {
        let mut out = vec![];
        if let Some(i) = self.entrance_info(server) { out.push((self.inner.me.clone(), self.inner.name(), true, i)); }
        let window = self.inner.timing.poll * 4;
        self.inner.with_state(server, |s| {
            for (node, info) in &s.infos {
                if *node == self.inner.me { continue; }
                if let Some(e) = &info.entrance {
                    let online = s.last_ok.get(node).map(|t| t.elapsed() < window).unwrap_or(false);
                    out.push((node.clone(), info.name.clone(), online, e.clone()));
                }
            }
        });
        out
    }

    async fn open_entrance(&self, server: &str, port: u16) -> Open {
        let info = Arc::new(Mutex::new(EntranceInfo { state: "preparando".into(), detail: "Abriendo la entrada…".into(), ..Default::default() }));
        let upnp = Arc::new(tokio::sync::Mutex::new(None));
        let mut open = Open { port, tasks: vec![], info: info.clone(), _pm: None, upnp: upnp.clone() };
        let set = |st: &str, d: String| { let mut i = info.lock().unwrap(); i.state = st.into(); i.detail = d; };
        let test = test_bind();
        let Some(lan) = test.or_else(local_ipv4) else {
            set("error", "Esta PC no tiene conexión de red con salida a Internet.".into());
            return open;
        };
        let guard = Arc::new(Mutex::new(Guard::default()));
        let mut binds = vec![SocketAddr::from((lan, port))];
        let v6 = if test.is_none() { global_ipv6() } else { None };
        if let Some(v6) = v6 { binds.push(SocketAddr::from((v6, port))); }
        let mut bound = 0;
        for b in &binds {
            match TcpListener::bind(b).await {
                Ok(l) => {
                    bound += 1;
                    let (inner, sid, guard) = (self.inner.clone(), server.to_string(), guard.clone());
                    open.tasks.push(tokio::spawn(async move {
                        loop {
                            let Ok((tcp, peer)) = l.accept().await else { break };
                            // El interruptor se vuelve a mirar en cada conexión: apagado, no entra nadie
                            // aunque la escucha todavía no se haya cerrado.
                            if !access::internet_entrance(&sid) { drop(tcp); continue; }
                            if !guard.lock().unwrap().enter(peer.ip()) { drop(tcp); continue; }
                            let _ = tcp.set_nodelay(true);
                            let (inner, sid, guard) = (inner.clone(), sid.clone(), guard.clone());
                            tokio::spawn(async move {
                                let _ = inner.forward_external(&sid, tcp).await;
                                guard.lock().unwrap().leave(peer.ip());
                            });
                        }
                    }));
                }
                Err(e) if b.is_ipv4() => {
                    set("error", format!("No se pudo usar el puerto {port} en esta PC ({e}). Elegí otro puerto público para este servidor."));
                    return open;
                }
                Err(_) => {}
            }
        }
        if bound == 0 { set("error", format!("No se pudo usar el puerto {port} en esta PC.")); return open; }
        {
            let mut i = info.lock().unwrap();
            i.local = Some(format!("{lan}:{port}"));
            i.addr_v6 = v6.map(|ip| format!("[{ip}]:{port}"));
        }
        if test.is_some() {
            let mut i = info.lock().unwrap();
            i.addr = Some(format!("{lan}:{port}")); i.method = "manual".into(); i.state = "abierta".into(); i.detail = "Escuchando (prueba).".into();
        } else if host939_core::settings::load().public_entrance_mapping {
            // Router: UPnP primero (puerto fijo y con el nombre «939 Host»), si no NAT-PMP/PCP.
            match self.map_upnp(lan, port).await {
                Ok((gw, ext_ip, ext_port)) => {
                    *upnp.lock().await = Some((gw, ext_port));
                    self.note_mapping(&info, ext_ip, ext_port, "upnp");
                    let (upnp, info2, lan2) = (upnp.clone(), info.clone(), lan);
                    open.tasks.push(tokio::spawn(async move {
                        // Renovación (la regla vence a la hora si 939 Host se cierra de golpe).
                        loop {
                            tokio::time::sleep(Duration::from_secs(20 * 60)).await;
                            let g = upnp.lock().await;
                            if let Some((gw, ext)) = g.as_ref() {
                                if gw.add_port(igd_next::PortMappingProtocol::TCP, *ext, SocketAddr::from((lan2, port)), 3600, "939 Host").await.is_err() {
                                    let mut i = info2.lock().unwrap(); i.state = "error".into(); i.detail = "El router dejó de mantener el puerto abierto.".into();
                                }
                            }
                        }
                    }));
                }
                Err(_) => {
                    let pm = portmapper::Client::new(portmapper::Config { enable_upnp: false, enable_pcp: true, enable_nat_pmp: true, protocol: portmapper::Protocol::Tcp });
                    if let Some(p) = std::num::NonZeroU16::new(port) { pm.update_local_port(p); }
                    let mut w = pm.watch_external_address();
                    let got = tokio::time::timeout(Duration::from_secs(8), async { loop { if let Some(a) = *w.borrow() { return Some(a); } if w.changed().await.is_err() { return None; } } }).await.ok().flatten();
                    match got {
                        Some(a) => { self.note_mapping(&info, *a.ip(), a.port(), "pcp"); open._pm = Some(pm); }
                        None => self.manual(&info, lan, port),
                    }
                }
            }
        } else {
            self.manual(&info, lan, port);
        }
        // Chat de voz por esta misma entrada: Simple Voice Chat usa la IP con la que se entró y el
        // puerto UDP de voz del servidor. Si lo aloja otra PC del grupo, esta PC escucha ese puerto
        // UDP y lo lleva a la anfitriona por la red del grupo; si lo aloja esta PC, el propio mod
        // ya escucha. En ambos casos se abre ese puerto UDP en el router (UPnP) con el mismo número.
        {
            let (node, sid) = (self.clone(), server.to_string());
            open.tasks.push(tokio::spawn(async move { node.voice_watch(&sid, lan).await; }));
            let (node, sid, info4) = (self.clone(), server.to_string(), info.clone());
            let mapping = test.is_none() && host939_core::settings::load().public_entrance_mapping;
            open.tasks.push(tokio::spawn(async move {
                let mut mapped: Option<(u16, Instant)> = None;
                loop {
                    let vp = node.current_voice_port(&sid);
                    match vp {
                        None => { info4.lock().unwrap().voice = None; }
                        Some(vp) if mapped.map(|(p, t)| p != vp || t.elapsed() > Duration::from_secs(20 * 60)).unwrap_or(true) => {
                            let msg = if !mapping { format!("Para el chat de voz, abrí en el router el puerto UDP {vp} hacia {lan}.") }
                                else { match node.map_udp(lan, vp).await {
                                    Ok(()) => { mapped = Some((vp, Instant::now())); format!("Chat de voz: puerto UDP {vp} abierto en el router.") }
                                    Err(e) => format!("No se pudo abrir el puerto UDP {vp} del chat de voz en el router ({e}). Abrilo a mano hacia {lan}."),
                                } };
                            info4.lock().unwrap().voice = Some(msg);
                        }
                        _ => {}
                    }
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
            }));
        }
        // Comprobación desde afuera al abrir y cada 10 minutos.
        let (node, sid, info3) = (self.clone(), server.to_string(), info.clone());
        open.tasks.push(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            loop {
                node.check_entrance(&sid, &info3).await;
                // Si todavía no se pudo comprobar (por ejemplo, la otra PC aún no estaba lista), se
                // vuelve a probar a los 30 s en lugar de esperar 10 minutos.
                let ok = info3.lock().unwrap().state == "comprobada";
                tokio::time::sleep(Duration::from_secs(if ok { 600 } else { 30 })).await;
            }
        }));
        open
    }

    /// Puerto UDP de voz del servidor ahora: el que informa la anfitriona del grupo o, si lo aloja
    /// esta PC, el de su Simple Voice Chat.
    pub(crate) fn current_voice_port(&self, server: &str) -> Option<u16> {
        let lease = lease::read(server).filter(|l| l.state == crate::LeaseState::Hosting);
        match lease {
            Some(l) if network::load_share(server).enabled && l.host != self.inner.me => self.inner.with_state(server, |s| s.infos.get(&l.host).and_then(|i| i.voice_port)),
            _ if self.inner.local.is_running(server) => host939_core::voice::port(server, self.inner.local.mc_port(server)),
            _ => None,
        }
    }

    /// Abre (o renueva) el puerto UDP de voz en el router con el mismo número externo.
    async fn map_udp(&self, lan: Ipv4Addr, port: u16) -> Result<()> {
        let gw = tokio::time::timeout(Duration::from_secs(5), igd_next::aio::tokio::search_gateway(igd_next::SearchOptions { timeout: Some(Duration::from_secs(4)), ..Default::default() })).await??;
        gw.add_port(igd_next::PortMappingProtocol::UDP, port, SocketAddr::from((lan, port)), 3600, "939 Host voz").await
            .map_err(|e| anyhow!("el router no lo permitió: {e}"))
    }

    async fn map_upnp(&self, lan: Ipv4Addr, port: u16) -> Result<(igd_next::aio::Gateway<igd_next::aio::tokio::Tokio>, Ipv4Addr, u16)> {
        let gw = tokio::time::timeout(Duration::from_secs(5), igd_next::aio::tokio::search_gateway(igd_next::SearchOptions { timeout: Some(Duration::from_secs(4)), ..Default::default() })).await??;
        let IpAddr::V4(ext) = gw.get_external_ip().await? else { bail!("sin IPv4 externa") };
        let local = SocketAddr::from((lan, port));
        let ext_port = match gw.add_port(igd_next::PortMappingProtocol::TCP, port, local, 3600, "939 Host").await {
            Ok(()) => port,
            // Ese puerto externo ya lo usa otra PC de la casa: el router elige otro.
            Err(_) => gw.add_any_port(igd_next::PortMappingProtocol::TCP, local, 3600, "939 Host").await?,
        };
        Ok((gw, ext, ext_port))
    }

    fn note_mapping(&self, info: &Arc<Mutex<EntranceInfo>>, ext: Ipv4Addr, port: u16, method: &str) {
        let mut i = info.lock().unwrap();
        i.method = method.into();
        if access::behind_cgnat(ext) {
            i.state = "cgnat".into();
            i.detail = format!("El router abrió el puerto, pero su IP de Internet ({ext}) es compartida por tu proveedor (CGNAT): desde Internet no se llega a esta PC. Otra PC del grupo puede ser la entrada.");
            i.addr = None;
        } else {
            i.state = "abierta".into();
            i.detail = "El router abrió el puerto. Comprobando desde afuera…".into();
            i.addr = Some(format!("{ext}:{port}"));
        }
    }

    /// Sin apertura automática: se muestra la IP pública que ve el grupo y cómo abrir el puerto.
    fn manual(&self, info: &Arc<Mutex<EntranceInfo>>, lan: Ipv4Addr, port: u16) {
        let public = self.inner.ep.addr().addrs.iter().filter_map(|t| match t { iroh::TransportAddr::Ip(SocketAddr::V4(a)) if access::is_global_v4(*a.ip()) => Some(*a.ip()), _ => None }).next();
        let mut i = info.lock().unwrap();
        i.method = "manual".into();
        i.addr = public.map(|ip| format!("{ip}:{port}"));
        i.state = "sin-acceso".into();
        i.detail = format!("El router no abrió el puerto solo (UPnP desactivado o no disponible). Para usar esta PC como entrada, abrí en el router el puerto TCP {port} hacia {lan}.");
    }

    /// Pide a otra PC del grupo que pruebe la entrada desde su conexión, como lo haría un jugador.
    async fn check_entrance(&self, server: &str, info: &Arc<Mutex<EntranceInfo>>) {
        let Some(addr) = info.lock().unwrap().addr.clone() else { return };
        let share = network::load_share(server);
        let window = self.inner.timing.poll * 4;
        let peers: Vec<String> = if share.enabled {
            share.members.iter().filter(|m| m.node_id != self.inner.me).filter(|m| self.inner.with_state(server, |s| s.last_ok.get(&m.node_id).map(|t| t.elapsed() < window).unwrap_or(false))).map(|m| m.node_id.clone()).collect()
        } else { vec![] };
        for p in peers {
            if let Ok(Resp::Probe { ok, detail }) = self.inner.rpc_timeout(&p, &Req::ProbeEntrance { server: server.into(), addr: addr.clone() }, Duration::from_secs(12)).await {
                let mut i = info.lock().unwrap();
                i.checked_at = Some(now());
                if ok { i.state = "comprobada".into(); i.detail = format!("Comprobada desde otra PC del grupo: {detail}."); }
                else if i.state != "cgnat" { i.state = "sin-acceso".into(); i.detail = format!("Otra PC del grupo no pudo entrar ({detail}). Revisá que el router tenga el puerto abierto y que el Firewall de Windows permita a 939 Host."); }
                return;
            }
        }
        let mut i = info.lock().unwrap();
        if i.state == "abierta" { i.detail = "Abierta, sin comprobar desde afuera: no hay otra PC del grupo conectada para probarla.".into(); }
    }
}

impl Inner {
    pub(crate) fn entrance_info(&self, server: &str) -> Option<EntranceInfo> {
        self.entrances.lock().unwrap().get(server).map(|o| o.info.lock().unwrap().clone())
    }

    /// Lleva a un jugador externo al servidor: a la anfitriona actual si es del grupo, o al
    /// Minecraft local si es un servidor de esta PC sola. Nunca a otro puerto.
    pub(crate) async fn forward_external(self: &Arc<Self>, server: &str, tcp: TcpStream) -> Result<()> {
        if network::load_share(server).enabled { return self.route_one(server, tcp).await; }
        if self.local.is_running(server) {
            let up = TcpStream::connect(("127.0.0.1", self.local.mc_port(server))).await?;
            let _ = up.set_nodelay(true);
            let (mut a, mut b) = (tcp, up);
            let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
            return Ok(());
        }
        moving::respond(tcp, &server_name(server), "El servidor está apagado. Volvé a intentar cuando lo enciendan.").await
    }

    /// Prueba pedida por otra PC del grupo: un ping de estado de Minecraft a su entrada pública.
    /// Solo a IPs públicas y puertos altos, con límite por PC: no sirve para escanear redes.
    pub(crate) async fn probe_for(&self, remote: &str, addr: &str) -> Resp {
        if !self.probe_limit.lock().unwrap().allow(remote) { return Resp::Err { msg: "Demasiadas comprobaciones seguidas.".into() }; }
        let Ok(sa) = addr.parse::<SocketAddr>() else { return Resp::Err { msg: "Dirección inválida.".into() } };
        let public = match sa.ip() { IpAddr::V4(ip) => access::is_global_v4(ip), IpAddr::V6(ip) => access::is_global_v6(ip) };
        #[cfg(feature = "pruebas")]
        let public = public || (std::env::var("H939_ENTRANCE_BIND").is_ok() && sa.ip().is_loopback());
        if !public || sa.port() < 1024 { return Resp::Err { msg: "Solo se comprueban direcciones públicas.".into() }; }
        let host = sa.ip().to_string();
        let r = tokio::task::spawn_blocking(move || host939_core::reach::status(&host, sa.port(), Duration::from_secs(6))).await;
        match r {
            Ok(Ok(s)) => Resp::Probe { ok: true, detail: format!("Minecraft {} responde en {} ms", s.version, s.latency_ms) },
            Ok(Err(e)) => Resp::Probe { ok: false, detail: e },
            Err(_) => Resp::Probe { ok: false, detail: "falló la prueba".into() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limites_por_ip() {
        let mut g = Guard::default();
        let a: IpAddr = "181.1.1.1".parse().unwrap();
        for _ in 0..PER_IP_ACTIVE { assert!(g.enter(a)); }
        assert!(!g.enter(a), "tope de conexiones a la vez");
        let b: IpAddr = "181.1.1.2".parse().unwrap();
        assert!(g.enter(b), "otra IP no se ve afectada");
        for _ in 0..PER_IP_ACTIVE { g.leave(a); }
        let mut n = PER_IP_ACTIVE;
        while g.enter(a) { g.leave(a); n += 1; if n > 1000 { break; } }
        assert_eq!(n, PER_IP_PER_MIN, "tope por minuto");
    }
}
