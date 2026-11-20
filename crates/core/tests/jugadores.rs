//! Lista de jugadores: una persona por UUID (cambios de nombre), última vez vista correcta y
//! historial respaldado por la consola. Archivo aparte: `paths::init` fija la raíz por proceso.
use host939_core::events::NoEvents;
use host939_core::model::*;
use host939_core::process::Manager;
use host939_core::{instance, paths, players, sessions};
use std::sync::Arc;

#[tokio::test]
async fn una_persona_por_uuid_y_ultima_vez_vista_real() {
    let tmp = tempfile::tempdir().unwrap();
    paths::init(tmp.path().to_path_buf()).unwrap();
    let cfg = instance::create(CreateServerInput { name: "Jugadores".into(), description: String::new(), mc_version: "26.2".into(), loader: Loader::Vanilla, memory_mb: 1024,
        difficulty: "normal".into(), gamemode: "survival".into(), max_players: 4, port: 25931, online_mode: true, whitelist: true, view_distance: 4,
        simulation_distance: 4, level_seed: String::new(), accept_eula: true, import_world_path: None, optimize: vec![] }, 8192).await.unwrap();
    let run = paths::server_run_dir(&cfg.id);
    let u = "dc93628e-4a27-409a-8ed7-fa8b31818b75";
    // Cambió de nombre: la lista blanca tiene el viejo y la caché de Minecraft el nuevo.
    std::fs::write(run.join("whitelist.json"), format!(r#"[{{"uuid":"{u}","name":"Viejo"}}]"#)).unwrap();
    std::fs::write(run.join("ops.json"), format!(r#"[{{"uuid":"{}","name":"Viejo","level":4}}]"#, u.to_uppercase())).unwrap();
    std::fs::write(run.join("usercache.json"), format!(r#"[{{"name":"Nuevo","uuid":"{u}","expiresOn":"2026-12-15 10:00:00 -0300"}},{{"name":"Otra","uuid":"11111111-2222-4333-8444-555555555555","expiresOn":"2026-12-01 09:00:00 -0300"}}]"#)).unwrap();
    let mgr = Manager::new(Arc::new(NoEvents));
    let list = players::list(&mgr, &cfg);
    assert_eq!(list.len(), 2, "sin duplicados: {list:?}");
    let p = list.iter().find(|p| p.uuid.eq_ignore_ascii_case(u)).unwrap();
    assert_eq!(p.name, "Nuevo");
    assert!(p.whitelisted && p.op_level == Some(4));
    // «expiresOn» es entrada + 1 mes: la última entrada fue el 15/11, no el 15/12.
    let seen = chrono::DateTime::parse_from_rfc3339(p.last_seen.as_deref().unwrap()).unwrap();
    assert_eq!(seen.with_timezone(&chrono::FixedOffset::west_opt(3 * 3600).unwrap()).format("%Y-%m-%d %H:%M").to_string(), "2026-11-15 10:00");
    // Una salida registrada más nueva gana.
    sessions::record(&cfg.id, &sessions::Session { n: "Nuevo".into(), p: Some("Viejo".into()), j: "2026-11-19T20:00:00-03:00".into(), l: "2026-11-19T21:30:00-03:00".into(), cut: false });
    let p = players::list(&mgr, &cfg).into_iter().find(|p| p.name == "Nuevo").unwrap();
    assert_eq!(p.last_seen.as_deref(), Some("2026-11-19T21:30:00-03:00"));
    let h = sessions::history(&cfg.id, "Nuevo");
    assert_eq!((h.sessions, h.total_secs), (1, 5400));
    assert_eq!(h.other_names, vec!["Viejo".to_string()]);

    // Edición con el jugador desconectado: cambia solo esos valores, guarda la copia previa,
    // no deja la saturación por encima del hambre y queda en la auditoría.
    use host939_core::playeredit::{apply_offline, Edit};
    let mut d = vec![10, 0, 0];
    let f = |d: &mut Vec<u8>, k: &str, t: u8, v: &[u8]| { d.push(t); d.extend((k.len() as u16).to_be_bytes()); d.extend(k.as_bytes()); d.extend(v); };
    f(&mut d, "Health", 5, &6.0f32.to_be_bytes());
    f(&mut d, "foodLevel", 3, &18i32.to_be_bytes());
    f(&mut d, "foodSaturationLevel", 5, &5.0f32.to_be_bytes());
    f(&mut d, "XpLevel", 3, &2i32.to_be_bytes());
    f(&mut d, "Dimension", 8, &[0, 19, b'm', b'i', b'n', b'e', b'c', b'r', b'a', b'f', b't', b':', b'o', b'v', b'e', b'r', b'w', b'o', b'r', b'l', b'd'][..]);
    d.push(0);
    let dir = run.join("world").join("playerdata");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{u}.dat"));
    let mut enc = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    std::io::Write::write_all(&mut enc, &d).unwrap();
    std::fs::write(&file, enc.finish().unwrap()).unwrap();
    let before = std::fs::read(&file).unwrap();
    let e = Edit { health: Some(20.0), food: Some(4), xp_level: Some(30), saturation: None };
    let msg = apply_offline(&mgr, &cfg, "Nuevo", u, &e, "prueba").unwrap();
    assert!(msg.contains("Nuevo"), "{msg}");
    let v = host939_core::nbt::read_nbt_file(&file).unwrap();
    assert_eq!((v["Health"].as_f64(), v["foodLevel"].as_i64(), v["XpLevel"].as_i64()), (Some(20.0), Some(4), Some(30)));
    assert_eq!(v["foodSaturationLevel"].as_f64(), Some(4.0), "saturación bajada al nuevo hambre");
    assert_eq!(v["Dimension"], "minecraft:overworld");
    assert_eq!(std::fs::read(file.with_extension("dat.antes-939host")).unwrap(), before, "copia previa intacta");
    let audit = std::fs::read_to_string(paths::server_dir(&cfg.id).join("audit.jsonl")).unwrap();
    assert!(audit.contains("jugador-editado Nuevo") && audit.contains("prueba"));
    assert!(apply_offline(&mgr, &cfg, "Nuevo", u, &Edit { health: Some(0.0), ..Default::default() }, "prueba").is_err(), "nunca mata");
    assert!(apply_offline(&mgr, &cfg, "Otra", "11111111-2222-4333-8444-555555555555", &e, "prueba").is_err(), "sin datos guardados");
}
