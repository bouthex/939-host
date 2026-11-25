//! Logo de 939 Host para servidores nuevos: persiste, viaja con la carpeta y no pisa logos propios.
//! Archivo aparte: `paths::init` fija la carpeta raíz una sola vez por proceso.
use host939_core::model::*;
use host939_core::{instance, paths};

fn input(name: &str, port: u16) -> CreateServerInput {
    CreateServerInput { name: name.into(), description: "logo".into(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port, online_mode: true, whitelist: false, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None, optimize: vec![] }
}

#[tokio::test]
async fn los_servidores_nuevos_traen_el_logo_de_939_y_la_copia_conserva_el_propio() {
    use host939_core::version_change::{duplicate, CopyParts};
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let cfg = instance::create(input("Con logo", 25925), 8192).await.unwrap();
    assert_eq!(cfg.logo.as_deref(), Some("939"));
    // Persiste: se relee de disco (como al reiniciar) y el archivo está en la carpeta del servidor,
    // que es la que viaja al grupo.
    assert_eq!(instance::load(&cfg.id).unwrap().logo.as_deref(), Some("939"));
    assert!(paths::server_dir(&cfg.id).join("logo.png").is_file());
    assert!(paths::server_run_dir(&cfg.id).join("server-icon.png").is_file());
    assert!(host939_core::logo::data_url(&cfg.id).is_some());
    // Un logo elegido después reemplaza al de 939 y la copia se lo lleva.
    let png = |s: u32| {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        v.extend_from_slice(b"IHDR"); v.extend_from_slice(&s.to_be_bytes()); v.extend_from_slice(&s.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]);
        v
    };
    host939_core::logo::set(&cfg.id, "preset:3", &png(256), &png(64)).unwrap();
    let id = duplicate(&cfg.id, "Copia con logo", &CopyParts { world: false, mods: false, config: false }, 8192).await.unwrap();
    assert_eq!(instance::load(&id).unwrap().logo.as_deref(), Some("preset:3"));
    assert_eq!(std::fs::read(paths::server_dir(&id).join("logo.png")).unwrap(), png(256));
    // Volver al ícono genérico sigue existiendo.
    host939_core::logo::clear(&id).unwrap();
    assert!(instance::load(&id).unwrap().logo.is_none());
}
