//! Papelera: borrar, restaurar completo, conflictos, borrado definitivo, vaciar y vencimiento.
use host939_core::events::{Events, InstallProgress};
use host939_core::model::*;
use host939_core::process::Manager;
use host939_core::{instance, paths, trash};
use std::sync::Arc;

struct NoEv;
impl Events for NoEv { fn server_changed(&self, _: &str) {} fn console_line(&self, _: &str, _: &str) {} fn install_progress(&self, _: &InstallProgress) {} }

fn input(name: &str, port: u16) -> CreateServerInput {
    CreateServerInput { name: name.into(), description: "papelera".into(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port, online_mode: true, whitelist: false, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None }
}

#[tokio::test]
async fn trash_full_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let mgr = Manager::new(Arc::new(NoEv));

    // Servidor con mundo, mods y configuración propios.
    let cfg = instance::create(input("Supervivencia", 25901), 8192).await.unwrap();
    let run = paths::server_run_dir(&cfg.id);
    std::fs::create_dir_all(run.join("world")).unwrap();
    std::fs::write(run.join("world/level.dat"), b"mundo-original").unwrap();
    std::fs::create_dir_all(run.join("mods")).unwrap();
    std::fs::write(run.join("mods/ejemplo.jar"), b"jar").unwrap();
    let code = cfg.identity.access_code.clone();

    instance::delete(&mgr, &cfg.id, "Supervivencia").unwrap();
    assert!(!paths::server_dir(&cfg.id).exists());
    let items = trash::list();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].meta.name, "Supervivencia");
    assert_eq!(items[0].days_left, 30);
    assert!(items[0].size_bytes > 0);
    assert!(!items[0].conflict);
    println!("✔ borrado a la Papelera: 30 días, {} bytes", items[0].size_bytes);

    // Restaurar: vuelve todo, con la misma identidad.
    let id = trash::restore(&items[0].key).unwrap();
    assert_eq!(id, cfg.id);
    let back = instance::load(&id).unwrap();
    assert_eq!(back.identity.access_code, code, "la identidad tiene que ser la misma");
    assert_eq!(std::fs::read(paths::server_run_dir(&id).join("world/level.dat")).unwrap(), b"mundo-original");
    assert!(paths::server_run_dir(&id).join("mods/ejemplo.jar").exists());
    assert!(trash::list().is_empty());
    println!("✔ restaurado completo: mundo, mods, configuración e identidad");

    // Conflicto: se borra, se crea otro con el mismo nombre y se restaura sin pisar nada.
    instance::delete(&mgr, &id, "Supervivencia").unwrap();
    let other = instance::create(input("Supervivencia", 25902), 8192).await.unwrap();
    std::fs::create_dir_all(paths::server_run_dir(&other.id).join("world")).unwrap();
    std::fs::write(paths::server_run_dir(&other.id).join("world/level.dat"), b"mundo-nuevo").unwrap();
    let it = trash::list().remove(0);
    assert!(it.conflict);
    let rid = trash::restore(&it.key).unwrap();
    assert_ne!(rid, other.id);
    assert_eq!(instance::load(&rid).unwrap().name, "Supervivencia (restaurado)");
    assert_ne!(instance::load(&rid).unwrap().identity.access_code, instance::load(&other.id).unwrap().identity.access_code, "identidades distintas");
    assert_eq!(std::fs::read(paths::server_run_dir(&other.id).join("world/level.dat")).unwrap(), b"mundo-nuevo", "el otro servidor no se toca");
    assert_eq!(std::fs::read(paths::server_run_dir(&rid).join("world/level.dat")).unwrap(), b"mundo-original");
    println!("✔ conflicto de nombre: restaurado como «Supervivencia (restaurado)» sin pisar el otro");

    // Borrado definitivo y vaciar.
    instance::delete(&mgr, &rid, "Supervivencia (restaurado)").unwrap();
    instance::delete(&mgr, &other.id, "Supervivencia").unwrap();
    assert_eq!(trash::list().len(), 2);
    let k = trash::list()[0].key.clone();
    trash::purge(&k).unwrap();
    assert_eq!(trash::list().len(), 1);
    let (n, bytes, errs) = trash::empty();
    assert_eq!((n, errs.len()), (1, 0));
    assert!(bytes > 0);
    assert!(trash::list().is_empty());
    println!("✔ borrado definitivo y «Vaciar papelera»");

    // Vencimiento: a los 30 días se borra solo, contando desde la fecha real de borrado.
    let c3 = instance::create(input("Viejo", 25903), 8192).await.unwrap();
    instance::delete(&mgr, &c3.id, "Viejo").unwrap();
    let it = trash::list().remove(0);
    let meta_path = paths::trash().join(&it.key).join(".939-papelera.json");
    let mut m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&meta_path).unwrap()).unwrap();
    m["deletedAt"] = serde_json::json!((chrono::Local::now() - chrono::Duration::days(29)).to_rfc3339());
    std::fs::write(&meta_path, m.to_string()).unwrap();
    assert_eq!(trash::purge_expired(), 0, "a los 29 días todavía se conserva");
    assert_eq!(trash::list()[0].days_left, 1);
    m["deletedAt"] = serde_json::json!((chrono::Local::now() - chrono::Duration::days(31)).to_rfc3339());
    std::fs::write(&meta_path, m.to_string()).unwrap();
    assert_eq!(trash::purge_expired(), 1);
    assert!(trash::list().is_empty());
    println!("✔ vencimiento a los 30 días desde la fecha real de borrado");
}
