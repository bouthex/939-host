//! Copia independiente de un servidor (opción «Crear una copia» del cambio de versión).
//! Papelera: borrar, restaurar completo, conflictos, borrado definitivo, vaciar y vencimiento.
use host939_core::model::*;
use host939_core::{instance, paths};



fn input(name: &str, port: u16) -> CreateServerInput {
    CreateServerInput { name: name.into(), description: "papelera".into(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port, online_mode: true, whitelist: false, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None }
}

#[tokio::test]
async fn duplicate_keeps_original_intact() {
    use host939_core::version_change::{duplicate, CopyParts};
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let cfg = instance::create(input("Original", 25921), 8192).await.unwrap();
    let run = paths::server_run_dir(&cfg.id);
    std::fs::create_dir_all(run.join("world")).unwrap();
    std::fs::write(run.join("world/level.dat"), b"mundo").unwrap();
    std::fs::write(run.join("world/session.lock"), b"x").unwrap();
    std::fs::create_dir_all(run.join("mods")).unwrap();
    std::fs::write(run.join("mods/a.jar"), b"jar").unwrap();
    let id = duplicate(&cfg.id, "Copia", &CopyParts { world: true, mods: false, config: true }, 8192).await.unwrap();
    let c = instance::load(&id).unwrap();
    assert_eq!(c.name, "Copia");
    assert_ne!(c.identity.access_code, cfg.identity.access_code);
    assert_ne!(c.port, cfg.port);
    let r2 = paths::server_run_dir(&id);
    assert_eq!(std::fs::read(r2.join("world/level.dat")).unwrap(), b"mundo");
    assert!(!r2.join("world/session.lock").exists());
    assert!(!r2.join("mods/a.jar").exists(), "los mods no se pidieron");
    assert!(run.join("mods/a.jar").exists() && run.join("world/level.dat").exists(), "el original queda intacto");
}
