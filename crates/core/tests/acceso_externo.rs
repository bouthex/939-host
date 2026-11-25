//! Interruptor «Permitir conexiones sin 939 Host» en un servidor de una sola PC: qué abre, qué
//! no, y que los ajustes del servidor no lo pierdan ni lo dejen sin modo online.
use host939_core::events::NoEvents;
use host939_core::model::*;
use host939_core::process::Manager;
use host939_core::{access, instance, paths, settings};
use std::sync::Arc;

#[tokio::test]
async fn interruptor_en_servidor_de_una_pc() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let cfg = instance::create(CreateServerInput { name: "Externo".into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port: 25941, online_mode: true, whitelist: false, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None, optimize: vec![] }, 8192).await.unwrap();
    let mgr = Manager::new(Arc::new(NoEvents));
    // Sin decidir: como antes (red local solo si la PC lo permite) y sin entrada pública.
    assert_eq!(access::external_allowed(&cfg.id), None);
    let mut st = settings::load();
    st.lan_access = true;
    st.public_entrance = true;
    settings::save(&st).unwrap();
    assert!(access::lan_effective(&cfg.id) && !access::internet_entrance(&cfg.id));
    // Permitido: entrada pública (la PC acepta) y red local.
    let mut c = instance::load(&cfg.id).unwrap();
    c.external_access = Some(true);
    instance::save(&c).unwrap();
    assert!(access::lan_effective(&cfg.id) && access::internet_entrance(&cfg.id));
    // Guardar los ajustes desde la interfaz (que no manda el interruptor) no lo pierde.
    let mut patch = instance::load(&cfg.id).unwrap();
    patch.external_access = None;
    patch.motd = "nuevo".into();
    instance::update(&mgr, patch, 8192).unwrap();
    assert_eq!(access::external_allowed(&cfg.id), Some(true));
    // Y no deja apagar el modo online mientras está permitido.
    let mut patch = instance::load(&cfg.id).unwrap();
    patch.online_mode = false;
    assert!(instance::update(&mgr, patch, 8192).unwrap_err().message.contains("Permitir conexiones sin 939 Host"));
    // La PC que no acepta ser entrada no la abre.
    st.public_entrance = false;
    settings::save(&st).unwrap();
    assert!(!access::internet_entrance(&cfg.id));
    // Bloqueado: ni red local ni entrada, aunque la PC lo permita.
    st.public_entrance = true;
    settings::save(&st).unwrap();
    let mut c = instance::load(&cfg.id).unwrap();
    c.external_access = Some(false);
    instance::save(&c).unwrap();
    assert!(!access::lan_effective(&cfg.id) && !access::internet_entrance(&cfg.id));
}
