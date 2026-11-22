//! 939publicar — herramienta del propietario de 939 Host para autorizar publicaciones.
//!
//! Corre en la PC del propietario, nunca en la CI. Crea la identidad de publicación (clave del
//! dispositivo dentro de Windows, no exportable, y una clave de recuperación para guardar fuera de la
//! PC), arma la publicación a partir de lo que compiló la CI, la bloquea si falta algo y la firma solo
//! con una confirmación explícita. No sube nada a internet: deja lista la carpeta `publicar/`.
//!
//! Uso: 939publicar <orden> --carpeta <carpeta-del-propietario> [opciones]
//!   iniciar      [--recuperacion <archivo>] [--sin-confirmacion-de-windows] [--solo-tpm]
//!   estado
//!   preparar     --artefactos <carpeta> --canal estable|beta --version X.Y.Z --evidencia <json> [--notas <texto>] [--datos N] [--vuelta-atras]
//!   autorizar    [--confirmo <versión>]
//!   rotar        [--con-recuperacion <archivo>]   (clave de dispositivo nueva; la anterior queda revocada)
//!   verificar    --raiz-inicial <archivo> --publicacion <archivo> [--instalada X.Y.Z]

use host939_update::keys::{PublicKey, SecretKey, Signer};
use host939_update::metadata::{Artifact, Envelope, Release, Root};
use host939_update::publish::{self, AuditEntry, Evidence, Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct Device {
    /// "cng-tpm" | "cng-software" | "archivo-solo-pruebas"
    kind: String,
    /// Nombre de la clave en Windows, o archivo (solo pruebas fuera de Windows).
    name: String,
    keyid: String,
    public: PublicKey,
}

#[derive(Serialize, Deserialize)]
struct OwnerState {
    root_version: u64,
    device: Device,
    recovery: PublicKey,
}

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned() }
fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }
fn die(msg: impl std::fmt::Display) -> ! { eprintln!("✘ {msg}"); std::process::exit(1) }
fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> T {
    serde_json::from_slice(&std::fs::read(p).unwrap_or_else(|e| die(format!("no se pudo leer {}: {e}", p.display())))).unwrap_or_else(|e| die(format!("{} no es válido: {e}", p.display())))
}
fn write(p: &Path, bytes: &[u8]) {
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let tmp = p.with_extension("tmp-939");
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, p)).unwrap_or_else(|e| die(format!("no se pudo escribir {}: {e}", p.display())));
}
fn sha_hex(b: &[u8]) -> String { hex::encode(Sha256::digest(b)) }

/// Abre el firmante del dispositivo.
fn device_signer(d: &Device, dir: &Path) -> Box<dyn Signer> {
    match d.kind.as_str() {
        #[cfg(windows)]
        "cng-tpm" | "cng-software" => {
            let store = if d.kind == "cng-tpm" { host939_update::cng::Store::Tpm } else { host939_update::cng::Store::Software };
            let k = host939_update::cng::CngKey::open(&d.name, store).unwrap_or_else(|e| die(format!("no se pudo abrir la clave del dispositivo: {e}")));
            if k.public() != d.public { die("la clave de Windows no coincide con la registrada"); }
            Box::new(k)
        }
        "archivo-solo-pruebas" => {
            let hexkey = std::fs::read_to_string(dir.join(&d.name)).unwrap_or_else(|e| die(e));
            let b = hex::decode(hexkey.trim()).unwrap_or_else(|e| die(e));
            let k = p256::ecdsa::SigningKey::from_slice(&b).unwrap_or_else(|e| die(e));
            Box::new(SecretKey::P256(k))
        }
        other => die(format!("tipo de clave no disponible en este sistema: {other}")),
    }
}

/// Crea una clave de dispositivo nueva con un nombre único.
fn new_device(dir: &Path, args: &[String]) -> (Device, Box<dyn Signer>) {
    let mut rnd = [0u8; 6];
    getrandom::getrandom(&mut rnd).unwrap();
    let name = format!("939Host-publicaciones-{}", hex::encode(rnd));
    #[cfg(windows)]
    {
        let _ = dir;
        let confirm = !flag(args, "--sin-confirmacion-de-windows");
        let k = host939_update::cng::CngKey::create(&name, confirm, flag(args, "--solo-tpm")).unwrap_or_else(|e| die(format!("Windows no pudo crear la clave: {e}")));
        if k.try_export_private() { die("Windows permitió exportar la clave privada: no se usa"); }
        let kind = if k.store == host939_update::cng::Store::Tpm { "cng-tpm" } else { "cng-software" };
        println!("✔ Clave del dispositivo creada en el {} (no exportable{}).", k.store.label(), if confirm { "; Windows pide confirmación en cada uso" } else { "" });
        let public = k.public();
        (Device { kind: kind.into(), name, keyid: public.key_id(), public }, Box::new(k))
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        // Fuera de Windows no hay almacén no exportable: solo para pruebas automáticas.
        let k = SecretKey::generate_p256();
        let SecretKey::P256(inner) = &k else { unreachable!() };
        let file = format!("{name}.SOLO-PRUEBAS.key");
        write(&dir.join(&file), hex::encode(inner.to_bytes()).as_bytes());
        println!("⚠ Clave de dispositivo en archivo ({file}): SOLO PARA PRUEBAS, fuera de Windows no hay almacén no exportable.");
        let public = k.public();
        (Device { kind: "archivo-solo-pruebas".into(), name: file, keyid: public.key_id(), public }, Box::new(k))
    }
}

fn recovery_from_file(p: &Path) -> SecretKey {
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| die(format!("no se pudo leer la recuperación: {e}")));
    let line = text.lines().find(|l| l.starts_with("CLAVE:")).unwrap_or_else(|| die("el archivo de recuperación no tiene la línea CLAVE:"));
    let raw: String = line["CLAVE:".len()..].chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let seed = data_encoding::BASE32_NOPAD.decode(raw.as_bytes()).unwrap_or_else(|_| die("la clave de recuperación está mal copiada"));
    let seed: [u8; 32] = seed.try_into().unwrap_or_else(|_| die("la clave de recuperación está incompleta"));
    SecretKey::ed25519_from_seed(&seed)
}

fn load_root(dir: &Path, version: u64) -> Root {
    let bytes = std::fs::read(dir.join("publicar").join(format!("raiz-{version}.json"))).unwrap_or_else(|e| die(e));
    let env = Envelope::parse(&bytes).unwrap_or_else(|e| die(e));
    serde_json::from_slice(&env.payload_bytes().unwrap()).unwrap_or_else(|e| die(e))
}

fn audit(dir: &Path, e: &AuditEntry) {
    let p = dir.join("publicaciones.jsonl");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).unwrap_or_else(|e| die(e));
    writeln!(f, "{}", serde_json::to_string(e).unwrap()).unwrap_or_else(|e| die(e));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_default();
    let dir = PathBuf::from(arg(&args, "--carpeta").unwrap_or_else(|| if cmd == "verificar" { ".".into() } else { die("falta --carpeta <carpeta-del-propietario>") }));
    let state_path = dir.join("propietario.json");
    match cmd.as_str() {
        // Publicación desde GitHub (ver docs/ACTUALIZACIONES.md, «Publicar una versión»): la clave de
        // publicación vive como secreto de un entorno protegido de GitHub que solo se usa con la
        // aprobación del propietario; la de recuperación, fuera de la PC, firma las raíces.
        "iniciar-ci" => {
            let raiz = dir.join("raiz-1.json");
            if raiz.exists() { die("esta carpeta ya tiene una raíz"); }
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(e));
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).unwrap();
            let recovery = SecretKey::ed25519_from_seed(&seed);
            let mut ci_seed = [0u8; 32];
            getrandom::getrandom(&mut ci_seed).unwrap();
            let ci = SecretKey::ed25519_from_seed(&ci_seed);
            let code = data_encoding::BASE32_NOPAD.encode(&seed);
            let grouped: Vec<String> = code.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect();
            let text = format!(
                "939 Host — clave de recuperación de las actualizaciones\n\n\
                 Sirve para recuperar el control de las actualizaciones si la clave de publicación de GitHub\n\
                 se perdiera o quedara expuesta (con ella se firma una raíz nueva y la anterior queda revocada).\n\
                 Quien la tenga puede tomar el control de las actualizaciones: guardala FUERA de la PC (impresa\n\
                 y en un pendrive guardado en otro lugar) y borrá este archivo de la PC y de las descargas.\n\n\
                 Identificador: {}\nCLAVE: {}\n",
                recovery.public().key_id(), grouped.join("-"));
            write(&dir.join("RECUPERACION-939Host-guardar-fuera-de-la-PC.txt"), text.as_bytes());
            let root = publish::root(1, publish::expires_in(3650), &[recovery.public()], 1, &[ci.public()], 1);
            let signed = publish::sign_root(&root, &[&recovery]).unwrap_or_else(|e| die(e));
            Root::trusted_initial(&signed).unwrap_or_else(|e| die(format!("la raíz generada no se verifica: {e}")));
            write(&raiz, &signed);
            write(&dir.join("clave-publicacion.secreta"), hex::encode(ci_seed).as_bytes());
            println!("✔ raíz 1 (recuperación {}…, publicación {}…)", &recovery.public().key_id()[..12], &ci.public().key_id()[..12]);
        }
        // Comprueba, sin firmar ni mostrar nada secreto, que la clave del entorno sea la que autoriza
        // la raíz que viaja en la app (solo imprime los primeros caracteres del identificador público).
        "comprobar-clave" => {
            let seed = std::env::var("H939_CLAVE_PUBLICACION").unwrap_or_default();
            let Some(seed) = hex::decode(seed.trim()).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()) else { die("la clave de publicación falta o no tiene el formato esperado (64 caracteres hexadecimales)") };
            let key = SecretKey::ed25519_from_seed(&seed);
            let root_path = PathBuf::from(arg(&args, "--raiz").unwrap_or_else(|| die("falta --raiz")));
            let root_bytes = std::fs::read(&root_path).unwrap_or_else(|e| die(format!("{}: {e}", root_path.display())));
            let root_bytes = serde_json::from_slice::<serde_json::Value>(&root_bytes).ok().and_then(|v| v.get("raiz").cloned()).filter(|v| !v.is_null()).map(|v| serde_json::to_vec(&v).unwrap()).unwrap_or(root_bytes);
            let root = Root::trusted_initial(&root_bytes).unwrap_or_else(|e| die(format!("raíz inválida: {e}")));
            let id = key.public().key_id();
            if root.release.keyids.contains(&id) { println!("✔ La clave del entorno coincide con la clave pública de la app (publicación {}…).", &id[..12]); }
            else { die(format!("La clave del entorno NO coincide con la de la app (la del entorno es {}…, la app espera {}…).", &id[..12], root.release.keyids.first().map(|k| &k[..12]).unwrap_or("?"))); }
        }
        "firmar-ci" => {
            let seed = std::env::var("H939_CLAVE_PUBLICACION").unwrap_or_else(|_| die("falta la clave de publicación (H939_CLAVE_PUBLICACION)"));
            let seed: [u8; 32] = hex::decode(seed.trim()).ok().and_then(|b| b.try_into().ok()).unwrap_or_else(|| die("la clave de publicación no tiene el formato esperado"));
            let key = SecretKey::ed25519_from_seed(&seed);
            let root_path = PathBuf::from(arg(&args, "--raiz").unwrap_or_else(|| die("falta --raiz <distribucion.json o raiz-N.json>")));
            let root_bytes = std::fs::read(&root_path).unwrap_or_else(|e| die(format!("{}: {e}", root_path.display())));
            // distribucion.json trae la raíz dentro de «raiz»; también se acepta la raíz sola.
            let root_bytes = serde_json::from_slice::<serde_json::Value>(&root_bytes).ok().and_then(|v| v.get("raiz").cloned()).filter(|v| !v.is_null()).map(|v| serde_json::to_vec(&v).unwrap()).unwrap_or(root_bytes);
            let mut root = Root::trusted_initial(&root_bytes).unwrap_or_else(|e| die(format!("raíz inválida: {e}")));
            for extra in args.iter().enumerate().filter(|(_, a)| *a == "--raiz-siguiente").filter_map(|(i, _)| args.get(i + 1)) {
                root = root.verify_next(&std::fs::read(extra).unwrap_or_else(|e| die(e))).unwrap_or_else(|e| die(format!("{extra}: {e}")));
            }
            if !root.release.keyids.contains(&key.public().key_id()) { die("la clave de publicación no es la que autoriza esta raíz"); }
            let version = arg(&args, "--version").unwrap_or_else(|| die("falta --version"));
            let v = semver::Version::parse(&version).unwrap_or_else(|e| die(format!("versión inválida: {e}")));
            let channel = if v.pre.is_empty() { "estable" } else { "beta" };
            let art_dir = PathBuf::from(arg(&args, "--artefactos").unwrap_or_else(|| die("falta --artefactos")));
            let mut artifacts = vec![];
            for n in [format!("939Host-{version}-instalador.exe"), format!("939Host-{version}-portable.zip")] {
                let b = std::fs::read(art_dir.join(&n)).unwrap_or_else(|e| die(format!("falta {n}: {e}")));
                artifacts.push(Artifact { name: n, size: b.len() as u64, sha256: sha_hex(&b) });
            }
            let now = chrono::Utc::now();
            let release = Release {
                kind: "release".into(), channel: channel.into(), seq: now.timestamp() as u64, published: now,
                expires: now + chrono::Duration::days(arg(&args, "--dias").and_then(|d| d.parse().ok()).unwrap_or(730)),
                version: version.clone(), data_version: arg(&args, "--datos").and_then(|d| d.parse().ok()).unwrap_or(1),
                rollback: false, artifacts, notes: arg(&args, "--notas").unwrap_or_default(),
            };
            let signed = publish::sign_release(&release, &[&key]).unwrap_or_else(|e| die(e));
            root.verify_release(&signed, now).unwrap_or_else(|e| die(format!("la publicación firmada no se verifica: {e}")));
            let out = PathBuf::from(arg(&args, "--salida").unwrap_or_else(|| dir.display().to_string()));
            write(&out.join(format!("{channel}.json")), &signed);
            println!("✔ {channel}.json firmado: 939 Host {version} (secuencia {})", release.seq);
        }
        "iniciar" => {
            if state_path.exists() { die("esta carpeta ya tiene una identidad de publicación (usá «rotar» para cambiar de clave)"); }
            std::fs::create_dir_all(&dir).unwrap_or_else(|e| die(e));
            let (device, signer) = new_device(&dir, &args);
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).unwrap();
            let recovery = SecretKey::ed25519_from_seed(&seed);
            let code = data_encoding::BASE32_NOPAD.encode(&seed);
            let grouped: Vec<String> = code.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect();
            let rec_path = arg(&args, "--recuperacion").map(PathBuf::from).unwrap_or_else(|| dir.join("RECUPERACION-guardar-fuera-de-la-PC.txt"));
            let text = format!(
                "939 Host — clave de recuperación de publicaciones\n\n\
                 Con esta clave se recupera el control de las publicaciones si se pierde o se roba la PC.\n\
                 Quien la tenga puede publicar en nombre del proyecto: guardala FUERA de la PC (por ejemplo,\n\
                 impresa en papel y en un pendrive guardado en otro lugar) y borrá este archivo de la PC.\n\n\
                 Identificador: {}\nCLAVE: {}\n",
                recovery.public().key_id(), grouped.join("-"));
            write(&rec_path, text.as_bytes());
            let root = publish::root(1, publish::expires_in(365), &[device.public.clone(), recovery.public()], 1, &[device.public.clone()], 1);
            let signed = publish::sign_root(&root, &[signer.as_ref(), &recovery]).unwrap_or_else(|e| die(e));
            Root::trusted_initial(&signed).unwrap_or_else(|e| die(format!("la raíz generada no se verifica: {e}")));
            write(&dir.join("publicar/raiz-1.json"), &signed);
            write(&dir.join("politica.json"), &serde_json::to_vec_pretty(&Policy::default()).unwrap());
            let st = OwnerState { root_version: 1, device, recovery: recovery.public() };
            write(&state_path, &serde_json::to_vec_pretty(&st).unwrap());
            println!("✔ Identidad de publicación creada. Raíz: publicar/raiz-1.json (pública: es la que va dentro de la app).");
            println!("✔ Clave de recuperación en {} — guardala fuera de la PC y borrala de acá.", rec_path.display());
        }
        "estado" => {
            let st: OwnerState = read_json(&state_path);
            let root = load_root(&dir, st.root_version);
            println!("Raíz {} (vence {}), dispositivo {} ({}), recuperación {}", root.version, root.expires.format("%Y-%m-%d"), &st.device.keyid[..16], st.device.kind, &st.recovery.key_id()[..16]);
            for ch in ["estable", "beta"] {
                let p = dir.join("publicar").join(format!("{ch}.json"));
                match std::fs::read(&p) {
                    Ok(b) => match root.verify_release(&b, chrono::Utc::now()) {
                        Ok(r) => println!("{ch}: {} (secuencia {}, vence {})", r.version, r.seq, r.expires.format("%Y-%m-%d")),
                        Err(e) => println!("{ch}: {e}"),
                    },
                    Err(_) => println!("{ch}: sin publicaciones"),
                }
            }
        }
        "preparar" => {
            let st: OwnerState = read_json(&state_path);
            let policy: Policy = read_json(&dir.join("politica.json"));
            let channel = arg(&args, "--canal").unwrap_or_else(|| die("falta --canal estable|beta"));
            let version = arg(&args, "--version").unwrap_or_else(|| die("falta --version"));
            let art_dir = PathBuf::from(arg(&args, "--artefactos").unwrap_or_else(|| die("falta --artefactos")));
            let ev: Evidence = read_json(Path::new(&arg(&args, "--evidencia").unwrap_or_else(|| die("falta --evidencia <json>"))));
            let sums = std::fs::read_to_string(art_dir.join("SHA256SUMS.txt")).unwrap_or_default();
            let mut files = vec![];
            for e in std::fs::read_dir(&art_dir).unwrap_or_else(|e| die(e)).flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if !n.ends_with("-instalador.exe") { continue; }
                let b = std::fs::read(e.path()).unwrap_or_else(|e| die(e));
                files.push((Artifact { name: n, size: b.len() as u64, sha256: sha_hex(&b) }, sums.clone()));
            }
            let root = load_root(&dir, st.root_version);
            let prev_bytes = std::fs::read(dir.join("publicar").join(format!("{channel}.json"))).ok();
            let previous = prev_bytes.as_ref().and_then(|b| {
                // La anterior puede estar vencida: se lee igual (solo para comparar).
                let env = Envelope::parse(b).ok()?;
                serde_json::from_slice::<Release>(&env.payload_bytes().ok()?).ok()
            });
            let _ = root;
            let draft = Release {
                kind: "release".into(), channel: channel.clone(), seq: previous.as_ref().map(|p| p.seq + 1).unwrap_or(1),
                published: chrono::Utc::now(), expires: publish::expires_in(policy.release_days), version: version.clone(),
                data_version: arg(&args, "--datos").and_then(|d| d.parse().ok()).unwrap_or(1), rollback: flag(&args, "--vuelta-atras"),
                artifacts: files.iter().map(|(a, _)| a.clone()).collect(), notes: arg(&args, "--notas").unwrap_or_default(),
            };
            let blocks = publish::gate(&policy, &ev, previous.as_ref(), &draft, &files);
            write(&dir.join("borrador.json"), &serde_json::to_vec_pretty(&serde_json::json!({ "release": draft, "evidence": ev, "artifacts_dir": art_dir })).unwrap());
            println!("Borrador: {channel} {version} (secuencia {}), {} archivo(s), commit {}", draft.seq, draft.artifacts.len(), ev.commit);
            if blocks.is_empty() { println!("✔ Sin bloqueos: se puede autorizar."); } else { for b in &blocks { println!("✘ {b}"); } std::process::exit(2); }
        }
        "autorizar" => {
            let st: OwnerState = read_json(&state_path);
            let policy: Policy = read_json(&dir.join("politica.json"));
            let b: serde_json::Value = read_json(&dir.join("borrador.json"));
            let mut draft: Release = serde_json::from_value(b["release"].clone()).unwrap_or_else(|e| die(e));
            let ev: Evidence = serde_json::from_value(b["evidence"].clone()).unwrap_or_else(|e| die(e));
            let art_dir = PathBuf::from(b["artifacts_dir"].as_str().unwrap_or("."));
            // Se vuelve a medir todo: el borrador o los archivos pudieron cambiar desde «preparar».
            let sums = std::fs::read_to_string(art_dir.join("SHA256SUMS.txt")).unwrap_or_default();
            let files: Vec<(Artifact, String)> = draft.artifacts.iter().map(|a| {
                let bytes = std::fs::read(art_dir.join(&a.name)).unwrap_or_else(|e| die(format!("{}: {e}", a.name)));
                (Artifact { name: a.name.clone(), size: bytes.len() as u64, sha256: sha_hex(&bytes) }, sums.clone())
            }).collect();
            let previous = std::fs::read(dir.join("publicar").join(format!("{}.json", draft.channel))).ok()
                .and_then(|b| Envelope::parse(&b).ok()).and_then(|e| serde_json::from_slice::<Release>(&e.payload_bytes().ok()?).ok());
            let blocks = publish::gate(&policy, &ev, previous.as_ref(), &draft, &files);
            if !blocks.is_empty() { for b in &blocks { println!("✘ {b}"); } die("la publicación está bloqueada"); }
            println!("Vas a autorizar 939 Host {} en el canal {} (commit {}).", draft.version, draft.channel, ev.commit);
            for a in &draft.artifacts { println!("  {}  {}  {} bytes", a.sha256, a.name, a.size); }
            let typed = match arg(&args, "--confirmo") {
                Some(v) => v,
                None => { print!("Para confirmar, escribí la versión ({}): ", draft.version); std::io::stdout().flush().ok(); let mut l = String::new(); std::io::stdin().lock().read_line(&mut l).ok(); l.trim().to_string() }
            };
            if typed != draft.version { die("no se confirmó: no se firmó nada"); }
            draft.published = chrono::Utc::now();
            let signer = device_signer(&st.device, &dir);
            let signed = publish::sign_release(&draft, &[signer.as_ref()]).unwrap_or_else(|e| die(format!("no se pudo firmar: {e}")));
            let root = load_root(&dir, st.root_version);
            root.verify_release(&signed, chrono::Utc::now()).unwrap_or_else(|e| die(format!("la publicación firmada no se verifica: {e}")));
            write(&dir.join("publicar").join(format!("{}.json", draft.channel)), &signed);
            for a in &draft.artifacts { std::fs::copy(art_dir.join(&a.name), dir.join("publicar").join(&a.name)).unwrap_or_else(|e| die(e)); }
            audit(&dir, &AuditEntry { at: chrono::Utc::now(), action: if draft.rollback { "vuelta-atras".into() } else { "publicacion".into() }, channel: draft.channel.clone(), version: draft.version.clone(), seq: draft.seq, keyid: st.device.keyid.clone(), commit: ev.commit.clone(), artifacts: draft.artifacts.clone(), metadata_sha256: sha_hex(&signed) });
            let _ = std::fs::remove_file(dir.join("borrador.json"));
            println!("✔ Autorizada y firmada. Subí el contenido de publicar/ al lugar de distribución (nada se subió solo).");
        }
        "rotar" => {
            let mut st: OwnerState = read_json(&state_path);
            let old_root = load_root(&dir, st.root_version);
            let old: Box<dyn Signer> = match arg(&args, "--con-recuperacion") {
                Some(p) => {
                    let k = recovery_from_file(Path::new(&p));
                    if k.public() != st.recovery { die("esa no es la clave de recuperación de esta identidad"); }
                    Box::new(k)
                }
                None => device_signer(&st.device, &dir),
            };
            let (device, signer) = new_device(&dir, &args);
            let root = publish::root(old_root.version + 1, publish::expires_in(365), &[device.public.clone(), st.recovery.clone()], 1, &[device.public.clone()], 1);
            let signed = publish::sign_root(&root, &[old.as_ref(), signer.as_ref()]).unwrap_or_else(|e| die(e));
            old_root.verify_next(&signed).unwrap_or_else(|e| die(format!("la raíz nueva no se verifica: {e}")));
            write(&dir.join("publicar").join(format!("raiz-{}.json", root.version)), &signed);
            let revoked = std::mem::replace(&mut st.device, device);
            st.root_version = root.version;
            write(&state_path, &serde_json::to_vec_pretty(&st).unwrap());
            audit(&dir, &AuditEntry { at: chrono::Utc::now(), action: format!("rotacion: revocada {}", &revoked.keyid[..16]), channel: String::new(), version: String::new(), seq: root.version, keyid: st.device.keyid.clone(), commit: String::new(), artifacts: vec![], metadata_sha256: sha_hex(&signed) });
            println!("✔ Raíz {}: clave nueva del dispositivo; la anterior ({}) ya no puede publicar. Subí publicar/raiz-{}.json.", root.version, &revoked.keyid[..16], root.version);
            println!("  Volvé a autorizar la última publicación de cada canal con la clave nueva.");
        }
        "verificar" => {
            let initial = std::fs::read(arg(&args, "--raiz-inicial").unwrap_or_else(|| die("falta --raiz-inicial"))).unwrap_or_else(|e| die(e));
            let pubfile = PathBuf::from(arg(&args, "--publicacion").unwrap_or_else(|| die("falta --publicacion")));
            let base = pubfile.parent().unwrap_or(Path::new("."));
            let mut client = host939_update::Client::new(&initial, Default::default()).unwrap_or_else(|e| die(e));
            client.update_root(|n| std::fs::read(base.join(format!("raiz-{n}.json"))).ok(), chrono::Utc::now()).unwrap_or_else(|e| die(e));
            let bytes = std::fs::read(&pubfile).unwrap_or_else(|e| die(e));
            let r = client.root.verify_release(&bytes, chrono::Utc::now()).unwrap_or_else(|e| die(e));
            println!("✔ Válida con la raíz {}: {} {} (secuencia {})", client.root.version, r.channel, r.version, r.seq);
            for a in &r.artifacts {
                match std::fs::read(base.join(&a.name)) {
                    Ok(b) if b.len() as u64 == a.size && sha_hex(&b) == a.sha256 => println!("✔ {} coincide", a.name),
                    Ok(_) => die(format!("{} no coincide con lo firmado", a.name)),
                    Err(_) => println!("· {} no está en la carpeta", a.name),
                }
            }
            if let Some(inst) = arg(&args, "--instalada") {
                let channel = r.channel.clone();
                let d = client.decide(&channel, if channel == "estable" { Some(&bytes) } else { None }, if channel == "beta" { Some(&bytes) } else { None }, &inst, 1, chrono::Utc::now()).unwrap_or_else(|e| die(e));
                println!("Para una PC con {inst}: {d:?}");
            }
        }
        _ => { eprintln!("Órdenes: iniciar, estado, preparar, autorizar, rotar, verificar (ver el comienzo de este archivo)"); std::process::exit(2) }
    }
}
