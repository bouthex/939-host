//! Datos de ejemplo con las formas reales de la app (para la vista previa de la interfaz en
//! `ci/vista-previa`). Uso: cargo run -p host939-core --example datos_vista_previa > datos.json
fn main() {
    let settings = host939_core::settings::AppSettings::default();
    let cfg: host939_core::model::ServerConfig = serde_json::from_value(serde_json::json!({
        "id": "srv-demo", "name": "Supervivencia con amigos", "description": "Mundo de prueba", "mcVersion": "1.21.4", "loader": "fabric",
        "loaderVersion": "0.16.9", "javaMajor": 21, "memoryMb": 4096, "jvmProfile": "balanced", "extraJvmArgs": "", "port": 25565,
        "maxPlayers": 8, "motd": "Un servidor de 939 Host", "onlineMode": true, "whitelist": true, "difficulty": "normal", "gamemode": "survival",
        "hardcore": false, "pvp": true, "levelSeed": "", "levelType": "minecraft:normal", "viewDistance": 10, "simulationDistance": 8,
        "spawnProtection": 0, "allowFlight": false, "enableCommandBlock": false, "syncIntervalMin": 15, "restoreIntervalMin": 15,
        "installed": true, "installError": null, "eulaAcceptedAt": "2026-11-01T10:00:00Z", "createdAt": "2026-11-01T10:00:00Z",
        "identity": { "publicKey": "a", "accessCode": "SUPERV-1234" }, "extraProperties": {}
    })).unwrap();
    println!("{}", serde_json::json!({ "settings": settings, "config": cfg }));
}
