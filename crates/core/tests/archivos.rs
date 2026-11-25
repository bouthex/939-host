//! Administrador de archivos local: rutas confinadas, mundo protegido con el servidor encendido,
//! sin sobrescrituras, Papelera y restauración, importar y exportar, validación de .jar.
use host939_core::model::*;
use host939_core::{files, instance, paths, trash};

fn input(name: &str, port: u16) -> CreateServerInput {
    CreateServerInput { name: name.into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Fabric, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port, online_mode: true, whitelist: false, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None, optimize: vec![] }
}

#[tokio::test]
async fn administrador_de_archivos() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().join("datos")).unwrap();
    let cfg = instance::create(input("Archivos", 25931), 8192).await.unwrap();
    let id = cfg.id.clone();
    let run = paths::server_run_dir(&id);
    std::fs::create_dir_all(run.join("world/region")).unwrap();
    std::fs::write(run.join("world/level.dat"), b"mundo").unwrap();
    std::fs::write(run.join("world/region/r.0.0.mca"), b"region").unwrap();
    std::fs::write(run.join("world/session.lock"), b"x").unwrap();
    std::fs::create_dir_all(run.join("config")).unwrap();
    std::fs::write(run.join("config/a.toml"), b"a=1").unwrap();

    // Listar y tipos.
    let root = files::list(&id, "", false).unwrap();
    assert!(root.iter().any(|e| e.name == "world" && e.dir && e.kind == "world"));
    assert!(root.iter().any(|e| e.name == "config" && e.kind == "config"));
    // Rutas que salen de la carpeta.
    assert!(files::list(&id, "../", false).is_err());
    assert!(files::mkdir(&id, "../fuera", false).is_err());
    assert!(!tmp.path().join("datos/servers/fuera").exists());

    // Con el servidor encendido el mundo es de solo lectura y no se descarga.
    assert!(files::list(&id, "world", true).unwrap().iter().all(|e| e.locked));
    assert!(files::delete(&id, "world/region/r.0.0.mca", true).is_err());
    assert!(files::transfer(&id, "config/a.toml", "world/a.toml", false, false, true).is_err());
    assert!(files::read_chunk(&id, "world/level.dat", 0, 10, true).is_err());
    assert!(files::delete(&id, "world/session.lock", false).is_err(), "session.lock nunca se toca");
    // Fuera del mundo se puede trabajar con el servidor encendido.
    files::mkdir(&id, "config/extra", true).unwrap();

    // Copiar sin sobrescribir; reemplazar manda el anterior a la Papelera.
    files::transfer(&id, "config/a.toml", "config/b.toml", false, false, false).unwrap();
    std::fs::write(run.join("config/b.toml"), b"b=2").unwrap();
    let e = files::transfer(&id, "config/a.toml", "config/b.toml", false, false, false).unwrap_err();
    assert_eq!(e.code, "exists");
    assert_eq!(std::fs::read(run.join("config/b.toml")).unwrap(), b"b=2", "no se sobrescribe sin pedirlo");
    files::transfer(&id, "config/a.toml", "config/b.toml", false, true, false).unwrap();
    assert_eq!(std::fs::read(run.join("config/b.toml")).unwrap(), b"a=1");
    assert!(files::transfer(&id, "config", "config/extra/dentro", true, false, false).is_err(), "no se mueve dentro de sí misma");
    files::transfer(&id, "config/b.toml", "config/extra/b.toml", true, false, false).unwrap();
    assert!(run.join("config/extra/b.toml").exists() && !run.join("config/b.toml").exists());

    // Borrar va a la Papelera y se restaura en su lugar.
    files::delete(&id, "config/extra/b.toml", false).unwrap();
    assert!(!run.join("config/extra/b.toml").exists());
    let items: Vec<_> = trash::list().into_iter().filter(|i| i.meta.kind == "archivos").collect();
    assert!(items.iter().any(|i| i.meta.name == "config/extra/b.toml"), "{items:?}");
    let reemplazado = items.iter().any(|i| i.meta.name == "config/b.toml");
    assert!(reemplazado, "lo reemplazado también quedó en la Papelera");
    let k = items.iter().find(|i| i.meta.name == "config/extra/b.toml").unwrap().key.clone();
    trash::restore(&k).unwrap();
    assert_eq!(std::fs::read(run.join("config/extra/b.toml")).unwrap(), b"a=1");
    // Si el lugar está ocupado, vuelve como «(restaurado)».
    files::delete(&id, "config/a.toml", false).unwrap();
    std::fs::write(run.join("config/a.toml"), b"nuevo").unwrap();
    let k2 = trash::list().into_iter().find(|i| i.meta.name == "config/a.toml").unwrap().key;
    trash::restore(&k2).unwrap();
    assert_eq!(std::fs::read(run.join("config/a.toml")).unwrap(), b"nuevo");
    assert_eq!(std::fs::read(run.join("config/a (restaurado).toml")).unwrap(), b"a=1");

    // Importar y exportar, sin pisar nada en ninguno de los dos lados.
    let afuera = tmp.path().join("afuera");
    std::fs::create_dir_all(afuera.join("pack/data")).unwrap();
    std::fs::write(afuera.join("pack/pack.mcmeta"), b"{}").unwrap();
    files::import(&id, &[afuera.join("pack")], "world/datapacks", false, false).unwrap();
    assert!(run.join("world/datapacks/pack/pack.mcmeta").exists());
    assert_eq!(files::import(&id, &[afuera.join("pack")], "world/datapacks", false, false).unwrap_err().code, "exists");
    let x1 = files::export(&id, "config/a.toml", &afuera, false).unwrap();
    let x2 = files::export(&id, "config/a.toml", &afuera, false).unwrap();
    assert_ne!(x1, x2, "el segundo export no pisa al primero");

    // Búsqueda.
    assert!(files::search(&id, "pack.mc", false, 50).unwrap().iter().any(|e| e.path.ends_with("pack/pack.mcmeta")));

    // Subida en partes: temporal y confirmación.
    let t = files::upload_temp(&id, "config/subido.txt", false).unwrap();
    std::fs::write(&t, b"hola").unwrap();
    files::commit_upload(&id, "config/subido.txt", &t, false, false).unwrap();
    assert_eq!(std::fs::read(run.join("config/subido.txt")).unwrap(), b"hola");
    let (b, total) = files::read_chunk(&id, "config/subido.txt", 1, 2, false).unwrap();
    assert_eq!((b.as_slice(), total), (&b"ol"[..], 4));

    // Validación de .jar.
    let bad = afuera.join("roto.jar");
    std::fs::write(&bad, b"no es un zip").unwrap();
    assert!(!files::check_jar(&cfg, &bad).ok);
    let fab = afuera.join("fab.jar");
    { let f = std::fs::File::create(&fab).unwrap(); let mut z = zip::ZipWriter::new(f); z.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default()).unwrap();
      std::io::Write::write_all(&mut z, br#"{"id":"x","name":"Mod X","version":"1"}"#).unwrap(); z.finish().unwrap(); }
    let c = files::check_jar(&cfg, &fab);
    assert!(c.ok && c.loader.as_deref() == Some("fabric"), "{c:?}");
    println!("✔ administrador de archivos local: confinado, mundo protegido, sin sobrescrituras, Papelera, importar/exportar, subida en partes y .jar");
}
