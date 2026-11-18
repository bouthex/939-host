//! Administración remota de archivos: un integrante autorizado (propietaria o coanfitriona)
//! explora y modifica los archivos del servidor en la PC que lo aloja, por la conexión P2P
//! cifrada del grupo (protocolo propio `939host/files/1`, no un escritorio remoto).
//!
//! La PC anfitriona aplica las mismas reglas que el administrador local (`host939_core::files`):
//! rutas confinadas, mundo de solo lectura con el servidor encendido, sin sobrescribir sin
//! pedirlo, Papelera para lo borrado o reemplazado. Además: una sola operación de escritura a la
//! vez por servidor y nada durante un traspaso. Las transferencias van en partes de 1 MB; si se
//! cancelan o se corta la conexión, el temporal se descarta y no queda un archivo a medias.

use super::*;
use host939_core::files as lf;

pub const ALPN_FILES: &[u8] = b"939host/files/1";
const CHUNK: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub(crate) enum FileReq {
    List { server: String, path: String },
    Search { server: String, q: String },
    Mkdir { server: String, path: String },
    Delete { server: String, path: String },
    Transfer { server: String, from: String, to: String, mv: bool, replace: bool },
    Download { server: String, path: String },
    Upload { server: String, path: String, size: u64, replace: bool },
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileResp {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    entries: Vec<lf::Entry>,
    #[serde(default)]
    size: u64,
}
fn fail(e: host939_core::error::AppError) -> FileResp { FileResp { ok: false, code: Some(e.code.to_string()), error: Some(e.message), ..Default::default() } }
fn ok() -> FileResp { FileResp { ok: true, ..Default::default() } }

/// Una escritura a la vez por servidor (también contra el administrador local de esa PC).
pub fn write_lock(server: &str) -> Option<WriteGuard> {
    static BUSY: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    let set = BUSY.get_or_init(Default::default);
    if !set.lock().unwrap().insert(server.to_string()) { return None; }
    Some(WriteGuard(server.to_string(), set))
}
pub struct WriteGuard(String, &'static std::sync::Mutex<std::collections::HashSet<String>>);
impl Drop for WriteGuard { fn drop(&mut self) { self.1.lock().unwrap().remove(&self.0); } }

#[derive(Clone)]
pub(crate) struct FilesH(pub(crate) Arc<Inner>);
impl std::fmt::Debug for FilesH { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "files") } }
impl ProtocolHandler for FilesH {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let remote = conn.remote_id().to_string();
        loop {
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return Ok(()) };
            let inner = self.0.clone();
            let remote = remote.clone();
            tokio::spawn(async move {
                let Ok(req) = wire::read_msg::<_, FileReq>(&mut recv).await else { return };
                let _ = inner.serve_files(&remote, req, &mut send, &mut recv).await;
                let _ = send.finish();
                let _ = send.stopped().await;
            });
        }
    }
}

impl Inner {
    /// ¿Puede `remote` administrar archivos de `server` en esta PC? Devuelve si está encendido.
    fn files_allowed(&self, server: &str, remote: &str, write: bool) -> Result<bool, String> {
        let share = network::load_share(server);
        match share.member(remote).map(|m| m.role.as_str()) {
            Some("propietario") | Some("coanfitrion") => {}
            Some(_) => return Err("Solo la propietaria y las coanfitrionas pueden administrar archivos.".into()),
            None => return Err("Esa PC no es parte del grupo de este servidor.".into()),
        }
        if !lease::read(server).map(|l| l.held_by(&self.me)).unwrap_or(false) { return Err("Esta PC ya no aloja el servidor: los archivos se administran en la anfitriona.".into()); }
        if write && self.with_state(server, |s| s.busy.clone()).is_some() { return Err("El servidor se está pasando a otra PC: esperá a que termine.".into()); }
        Ok(self.local.is_running(server))
    }

    async fn serve_files(&self, remote: &str, req: FileReq, send: &mut iroh::endpoint::SendStream, recv: &mut iroh::endpoint::RecvStream) -> Result<()> {
        let write = !matches!(req, FileReq::List { .. } | FileReq::Search { .. } | FileReq::Download { .. });
        let server = match &req { FileReq::List { server, .. } | FileReq::Search { server, .. } | FileReq::Mkdir { server, .. } | FileReq::Delete { server, .. }
            | FileReq::Transfer { server, .. } | FileReq::Download { server, .. } | FileReq::Upload { server, .. } => server.clone() };
        if server.contains(['/', '\\', '.']) { return wire::write_msg(send, &FileResp { ok: false, error: Some("Servidor inválido".into()), ..Default::default() }).await; }
        let active = match self.files_allowed(&server, remote, write) {
            Ok(a) => a,
            Err(e) => return wire::write_msg(send, &FileResp { ok: false, error: Some(e), code: Some("denied".into()), ..Default::default() }).await,
        };
        let _g = if write {
            match write_lock(&server) { Some(g) => Some(g), None => return wire::write_msg(send, &FileResp { ok: false, error: Some("Hay otra operación con archivos en curso en este servidor.".into()), code: Some("busy".into()), ..Default::default() }).await }
        } else { None };
        let resp = match req {
            FileReq::List { path, .. } => lf::list(&server, &path, active).map(|entries| FileResp { ok: true, entries, ..Default::default() }).unwrap_or_else(fail),
            FileReq::Search { q, .. } => lf::search(&server, &q, active, 200).map(|entries| FileResp { ok: true, entries, ..Default::default() }).unwrap_or_else(fail),
            FileReq::Mkdir { path, .. } => lf::mkdir(&server, &path, active).map(|_| ok()).unwrap_or_else(fail),
            FileReq::Delete { path, .. } => { let r = lf::delete(&server, &path, active).map(|_| ok()).unwrap_or_else(fail); if r.ok { self.local.note(&server, &format!("Una PC del grupo mandó «{path}» a la Papelera.")); } r }
            FileReq::Transfer { from, to, mv, replace, .. } => lf::transfer(&server, &from, &to, mv, replace, active).map(|_| ok()).unwrap_or_else(fail),
            FileReq::Download { path, .. } => {
                // Primero el tamaño; después las partes. Si el cliente corta, se deja de leer.
                let first = match lf::read_chunk(&server, &path, 0, 0, active) { Ok((_, total)) => total, Err(e) => return wire::write_msg(send, &fail(e)).await };
                wire::write_msg(send, &FileResp { ok: true, size: first, ..Default::default() }).await?;
                let mut off = 0u64;
                while off < first {
                    let (b, _) = match lf::read_chunk(&server, &path, off, CHUNK, active) { Ok(x) => x, Err(_) => break };
                    if b.is_empty() { break; }
                    off += b.len() as u64;
                    wire::write_frame(send, &b).await?;
                }
                return Ok(());
            }
            FileReq::Upload { path, size, replace, .. } => {
                if let Err(e) = lf::check_upload_size(&server, size) { return wire::write_msg(send, &fail(e)).await; }
                let tmp = match lf::upload_temp(&server, &path, active) { Ok(t) => t, Err(e) => return wire::write_msg(send, &fail(e)).await };
                wire::write_msg(send, &ok()).await?;
                let mut got = 0u64;
                let res: Result<()> = async {
                    use tokio::io::AsyncWriteExt;
                    let mut f = tokio::fs::File::create(&tmp).await?;
                    while got < size {
                        let b = wire::read_frame(recv, CHUNK + 16).await?;
                        if b.is_empty() { bail!("subida cancelada"); }
                        got += b.len() as u64;
                        if got > size { bail!("llegaron más datos de los anunciados"); }
                        f.write_all(&b).await?;
                    }
                    f.sync_all().await?;
                    Ok(())
                }.await;
                match res {
                    Ok(()) => lf::commit_upload(&server, &path, &tmp, replace, self.local.is_running(&server)).map(|_| ok()).unwrap_or_else(fail),
                    Err(e) => { let _ = std::fs::remove_file(&tmp); FileResp { ok: false, error: Some(format!("La subida no terminó: {e}")), ..Default::default() } }
                }
            }
        };
        wire::write_msg(send, &resp).await
    }
}

/// Resultado de una operación remota para la interfaz.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteError { pub code: String, pub message: String }

impl GroupNode {
    /// PC que aloja el servidor ahora (a la que se le piden los archivos).
    pub fn files_host(&self, server: &str) -> Option<String> { lease::read(server).filter(|l| l.state == LeaseState::Hosting || l.held_by(&self.inner.me)).map(|l| l.host) }

    async fn files_call(&self, server: &str, req: &FileReq) -> Result<(FileResp, iroh::endpoint::SendStream, iroh::endpoint::RecvStream)> {
        let host = self.files_host(server).ok_or_else(|| anyhow!("Nadie está alojando este servidor ahora."))?;
        if host == self.inner.me { bail!("Esta PC aloja el servidor: se usa el administrador local."); }
        let c = self.inner.conn(&host, ALPN_FILES).await.context("no se pudo contactar a la PC anfitriona")?;
        let (mut send, mut recv) = c.open_bi().await?;
        wire::write_msg(&mut send, req).await?;
        let r: FileResp = wire::read_msg(&mut recv).await?;
        Ok((r, send, recv))
    }

    fn check(r: FileResp) -> Result<FileResp> {
        if r.ok { Ok(r) } else { Err(anyhow!("{}{}", r.code.map(|c| format!("[{c}] ")).unwrap_or_default(), r.error.unwrap_or_else(|| "error".into()))) }
    }

    pub async fn files_list(&self, server: &str, path: &str) -> Result<Vec<lf::Entry>> {
        let (r, mut s, _) = self.files_call(server, &FileReq::List { server: server.into(), path: path.into() }).await?;
        let _ = s.finish();
        Ok(Self::check(r)?.entries)
    }
    pub async fn files_search(&self, server: &str, q: &str) -> Result<Vec<lf::Entry>> {
        let (r, mut s, _) = self.files_call(server, &FileReq::Search { server: server.into(), q: q.into() }).await?;
        let _ = s.finish();
        Ok(Self::check(r)?.entries)
    }
    pub async fn files_mkdir(&self, server: &str, path: &str) -> Result<()> {
        let (r, mut s, _) = self.files_call(server, &FileReq::Mkdir { server: server.into(), path: path.into() }).await?;
        let _ = s.finish(); Self::check(r).map(|_| ())
    }
    pub async fn files_delete(&self, server: &str, path: &str) -> Result<()> {
        let (r, mut s, _) = self.files_call(server, &FileReq::Delete { server: server.into(), path: path.into() }).await?;
        let _ = s.finish(); Self::check(r).map(|_| ())
    }
    pub async fn files_transfer(&self, server: &str, from: &str, to: &str, mv: bool, replace: bool) -> Result<()> {
        let (r, mut s, _) = self.files_call(server, &FileReq::Transfer { server: server.into(), from: from.into(), to: to.into(), mv, replace }).await?;
        let _ = s.finish(); Self::check(r).map(|_| ())
    }

    /// Descarga un archivo a `dest` (temporal + renombrado al final). `progress(hecho, total)`
    /// devuelve `false` para cancelar.
    pub async fn files_download(&self, server: &str, path: &str, dest: &std::path::Path, mut progress: impl FnMut(u64, u64) -> bool + Send) -> Result<()> {
        let (r, mut s, mut recv) = self.files_call(server, &FileReq::Download { server: server.into(), path: path.into() }).await?;
        let _ = s.finish();
        let total = Self::check(r)?.size;
        let tmp = dest.with_extension("939-descarga.tmp");
        let res: Result<()> = async {
            use tokio::io::AsyncWriteExt;
            let mut f = tokio::fs::File::create(&tmp).await?;
            let mut got = 0u64;
            while got < total {
                let b = wire::read_frame(&mut recv, CHUNK + 16).await?;
                if b.is_empty() { bail!("la anfitriona cortó la descarga"); }
                got += b.len() as u64;
                f.write_all(&b).await?;
                if !progress(got, total) { bail!("Descarga cancelada."); }
            }
            f.sync_all().await?;
            Ok(())
        }.await;
        match res {
            Ok(()) => { std::fs::rename(&tmp, dest)?; Ok(()) }
            Err(e) => { let _ = std::fs::remove_file(&tmp); Err(e) }
        }
    }

    /// Sube un archivo de esta PC a `path` en el servidor. `progress` devuelve `false` para cancelar.
    pub async fn files_upload(&self, server: &str, src: &std::path::Path, path: &str, replace: bool, mut progress: impl FnMut(u64, u64) -> bool + Send) -> Result<()> {
        use tokio::io::AsyncReadExt;
        let size = std::fs::metadata(src)?.len();
        let (r, mut send, mut recv) = self.files_call(server, &FileReq::Upload { server: server.into(), path: path.into(), size, replace }).await?;
        Self::check(r)?;
        let mut f = tokio::fs::File::open(src).await?;
        let mut buf = vec![0u8; CHUNK];
        let mut sent = 0u64;
        while sent < size {
            let n = f.read(&mut buf).await?;
            if n == 0 { break; }
            wire::write_frame(&mut send, &buf[..n]).await?;
            sent += n as u64;
            if !progress(sent, size) { let _ = wire::write_frame(&mut send, &[]).await; let _ = send.finish(); bail!("Subida cancelada."); }
        }
        let _ = send.finish();
        let done: FileResp = wire::read_msg(&mut recv).await?;
        Self::check(done).map(|_| ())
    }
}
