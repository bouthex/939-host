//! La herramienta del propietario de punta a punta, en carpetas descartables: crear la identidad,
//! preparar (con bloqueos), autorizar solo con confirmación, verificar como lo haría la app, cambiar
//! de PC, perder la PC y recuperar con la clave de recuperación.

use std::path::Path;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_939publicar")).args(args).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    (o.status.success(), text)
}

fn evidence(dir: &Path, branch: &str, fail: Option<&str>) -> String {
    let mut jobs = serde_json::Map::new();
    for j in host939_update::publish::Policy::default().required_jobs {
        let v = if Some(j.as_str()) == fail { "failure" } else { "success" };
        jobs.insert(j, v.into());
    }
    let p = dir.join(format!("evidencia-{branch}-{}.json", fail.is_some()));
    std::fs::write(&p, serde_json::json!({ "commit": "c".repeat(40), "branch": branch, "jobs": jobs, "authenticode": "NotSigned" }).to_string()).unwrap();
    p.to_string_lossy().to_string()
}

fn artifacts(dir: &Path, version: &str, content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = dir.join(format!("dist-{version}"));
    std::fs::create_dir_all(&d).unwrap();
    let name = format!("939Host-{version}-instalador.exe");
    std::fs::write(d.join(&name), content).unwrap();
    std::fs::write(d.join("SHA256SUMS.txt"), format!("{}  {name}\n", hex::encode(Sha256::digest(content)))).unwrap();
    d.to_string_lossy().to_string()
}

#[test]
fn identidad_publicacion_rotacion_y_recuperacion() {
    let tmp = tempfile::tempdir().unwrap();
    let owner = tmp.path().join("propietario");
    let o = owner.to_str().unwrap();
    let rec = tmp.path().join("pendrive").join("recuperacion.txt");
    let (ok, out) = run(&["iniciar", "--carpeta", o, "--recuperacion", rec.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(std::fs::read_to_string(&rec).unwrap().contains("CLAVE:"));
    let (ok, out) = run(&["iniciar", "--carpeta", o]);
    assert!(!ok && out.contains("ya tiene"), "no se pisa una identidad existente: {out}");
    // La raíz pública no lleva secretos.
    let root1 = std::fs::read_to_string(owner.join("publicar/raiz-1.json")).unwrap();
    let seed_line = std::fs::read_to_string(&rec).unwrap().lines().find(|l| l.starts_with("CLAVE:")).unwrap().to_string();
    assert!(!root1.contains(seed_line[6..].trim()), "la raíz no contiene la recuperación");

    // Bloqueos: prueba fallada, rama equivocada.
    let dist = artifacts(tmp.path(), "1.0.0", b"instalador 1.0.0");
    let (ok, out) = run(&["preparar", "--carpeta", o, "--artefactos", &dist, "--canal", "estable", "--version", "1.0.0", "--evidencia", &evidence(tmp.path(), "main", Some("Compilar instalador y portable"))]);
    assert!(!ok && out.contains("«Compilar instalador y portable»: failure"), "{out}");
    let (ok, out) = run(&["preparar", "--carpeta", o, "--artefactos", &dist, "--canal", "estable", "--version", "1.0.0", "--evidencia", &evidence(tmp.path(), "rediseno-v3", None)]);
    assert!(!ok && out.contains("rama"), "{out}");
    // Bien preparado; autorizar sin la confirmación correcta no firma nada.
    let (ok, out) = run(&["preparar", "--carpeta", o, "--artefactos", &dist, "--canal", "estable", "--version", "1.0.0", "--evidencia", &evidence(tmp.path(), "main", None)]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["autorizar", "--carpeta", o, "--confirmo", "9.9.9"]);
    assert!(!ok && out.contains("no se confirmó"), "{out}");
    assert!(!owner.join("publicar/estable.json").exists());
    // Si el instalador cambia entre preparar y autorizar, se bloquea.
    std::fs::write(Path::new(&dist).join("939Host-1.0.0-instalador.exe"), b"cambiado").unwrap();
    let (ok, out) = run(&["autorizar", "--carpeta", o, "--confirmo", "1.0.0"]);
    assert!(!ok && out.contains("bloqueada"), "{out}");
    std::fs::write(Path::new(&dist).join("939Host-1.0.0-instalador.exe"), b"instalador 1.0.0").unwrap();
    let (ok, out) = run(&["autorizar", "--carpeta", o, "--confirmo", "1.0.0"]);
    assert!(ok, "{out}");
    let log = std::fs::read_to_string(owner.join("publicaciones.jsonl")).unwrap();
    assert!(log.contains("\"version\":\"1.0.0\"") && log.contains(&"c".repeat(40)), "registro: {log}");
    // La app (verificar) la acepta desde la raíz 1.
    let pubdir = owner.join("publicar");
    let raiz1 = pubdir.join("raiz-1.json").to_string_lossy().to_string();
    let (ok, out) = run(&["verificar", "--raiz-inicial", &raiz1, "--publicacion", pubdir.join("estable.json").to_str().unwrap(), "--instalada", "0.9.0"]);
    assert!(ok && out.contains("coincide") && out.contains("Offer"), "{out}");
    // No se puede volver a publicar la misma versión.
    let (ok, out) = run(&["preparar", "--carpeta", o, "--artefactos", &dist, "--canal", "estable", "--version", "1.0.0", "--evidencia", &evidence(tmp.path(), "main", None)]);
    assert!(!ok && out.contains("no es mayor"), "{out}");

    // Cambio de PC normal y luego PC perdida: recuperación.
    let (ok, out) = run(&["rotar", "--carpeta", o]);
    assert!(ok, "{out}");
    let (ok, out) = run(&["rotar", "--carpeta", o, "--con-recuperacion", rec.to_str().unwrap()]);
    assert!(ok, "{out}");
    let otra = tmp.path().join("otra.txt");
    std::fs::write(&otra, std::fs::read_to_string(&rec).unwrap().replace(&seed_line, "CLAVE: AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA-AAAA")).unwrap();
    let (ok, out) = run(&["rotar", "--carpeta", o, "--con-recuperacion", otra.to_str().unwrap()]);
    assert!(!ok, "una recuperación ajena no sirve: {out}");
    // La publicación firmada con la clave revocada ya no vale con la raíz 3.
    let (ok, out) = run(&["verificar", "--raiz-inicial", &raiz1, "--publicacion", pubdir.join("estable.json").to_str().unwrap()]);
    assert!(!ok && out.contains("firmas insuficientes"), "{out}");
    // Con la clave nueva se publica la siguiente versión y la app la acepta encadenando 1 → 2 → 3.
    let dist2 = artifacts(tmp.path(), "1.0.1", b"instalador 1.0.1");
    assert!(run(&["preparar", "--carpeta", o, "--artefactos", &dist2, "--canal", "estable", "--version", "1.0.1", "--evidencia", &evidence(tmp.path(), "main", None)]).0);
    assert!(run(&["autorizar", "--carpeta", o, "--confirmo", "1.0.1"]).0);
    let (ok, out) = run(&["verificar", "--raiz-inicial", &raiz1, "--publicacion", pubdir.join("estable.json").to_str().unwrap()]);
    assert!(ok && out.contains("raíz 3") && out.contains("1.0.1"), "{out}");
}
