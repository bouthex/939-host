//! Detección de hardware: evaluación inicial, métricas livianas y prueba profunda a pedido.

use crate::error::{AppError, AppResult};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Instant;
use sysinfo::{CpuRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// Un único `System` reutilizado: evita reservar memoria en cada consulta.
pub struct Probe(Mutex<System>);

impl Probe {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing().with_frequency()).with_memory(MemoryRefreshKind::nothing().with_ram()),
        );
        Probe(Mutex::new(sys))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub kind: String,
    pub total_gb: f64,
    pub free_gb: f64,
    pub is_data_disk: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareSnapshot {
    pub cpu_model: String,
    pub physical_cores: Option<usize>,
    pub logical_cores: usize,
    pub cpu_mhz: u64,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub os: String,
    pub host_name: String,
    pub disks: Vec<DiskInfo>,
    pub benchmark: Option<BenchmarkResult>,
    pub assessment: Assessment,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveMetrics {
    pub cpu_usage: f32,
    pub ram_available_mb: u64,
    pub ram_total_mb: u64,
    pub host_ready: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResult {
    /// Puntos de CPU por núcleo (más es mejor; comparables entre PCs con 939 Host).
    pub cpu_single_score: u32,
    pub disk_write_mb_s: Option<u32>,
    pub measured_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assessment {
    pub rating: String,
    pub reasons: Vec<String>,
    pub recommended_server_ram_mb: u32,
    pub max_server_ram_mb: u32,
}

fn bench_path() -> std::path::PathBuf { paths::identity().join("benchmark.json") }

pub fn saved_benchmark() -> Option<BenchmarkResult> { paths::read_json(&bench_path()).ok() }

pub fn snapshot(probe: &Probe) -> HardwareSnapshot {
    let mut sys = probe.0.lock().unwrap();
    sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
    sys.refresh_cpu_specifics(CpuRefreshKind::nothing().with_frequency());
    let cpu_model = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_else(|| "Desconocido".into());
    let cpu_mhz = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
    let logical = sys.cpus().len();
    let physical = sys.physical_core_count();
    let total = sys.total_memory() / 1_048_576;
    let avail = sys.available_memory() / 1_048_576;
    drop(sys);

    let data_root = paths::root().to_string_lossy().to_lowercase();
    let disks = Disks::new_with_refreshed_list();
    let mut disk_list: Vec<DiskInfo> = disks
        .list()
        .iter()
        .map(|d| {
            let mount = d.mount_point().to_string_lossy().to_string();
            DiskInfo {
                name: d.name().to_string_lossy().to_string(),
                kind: match d.kind() {
                    sysinfo::DiskKind::SSD => "SSD".into(),
                    sysinfo::DiskKind::HDD => "HDD".into(),
                    _ => "Desconocido".into(),
                },
                total_gb: d.total_space() as f64 / 1e9,
                free_gb: d.available_space() as f64 / 1e9,
                is_data_disk: !mount.is_empty() && data_root.starts_with(&mount.to_lowercase()),
                mount,
            }
        })
        .collect();
    disk_list.sort_by(|a, b| b.is_data_disk.cmp(&a.is_data_disk));

    let benchmark = saved_benchmark();
    let assessment = assess(total, disk_list.iter().find(|d| d.is_data_disk), benchmark.as_ref());
    HardwareSnapshot {
        cpu_model,
        physical_cores: physical,
        logical_cores: logical,
        cpu_mhz,
        ram_total_mb: total,
        ram_available_mb: avail,
        os: System::long_os_version().unwrap_or_else(|| "Windows".into()),
        host_name: System::host_name().unwrap_or_else(|| "Esta PC".into()),
        disks: disk_list,
        benchmark,
        assessment,
    }
}

fn assess(ram_total_mb: u64, data_disk: Option<&DiskInfo>, bench: Option<&BenchmarkResult>) -> Assessment {
    let mut reasons = vec![];
    let mut level = 3; // 3 buena, 2 aceptable, 1 limitada
    let gb = ram_total_mb as f64 / 1024.0;
    if gb < 8.0 {
        level = 1;
        reasons.push(format!("{gb:.0} GB de RAM: alcanza para Vanilla o pocos mods."));
    } else if gb < 16.0 {
        level = level.min(2);
        reasons.push(format!("{gb:.0} GB de RAM: bien para modpacks medianos."));
    } else {
        reasons.push(format!("{gb:.0} GB de RAM: margen para modpacks grandes."));
    }
    match data_disk {
        Some(d) if d.kind == "HDD" => { level = level.min(2); reasons.push("Los datos están en un disco HDD: el guardado y la carga de chunks son más lentos que en SSD.".into()); }
        Some(d) if d.free_gb < 10.0 => { level = level.min(1); reasons.push(format!("Quedan {:.0} GB libres en el disco de datos.", d.free_gb)); }
        Some(d) if d.kind == "SSD" => reasons.push("Los datos están en un SSD.".into()),
        _ => {}
    }
    match bench {
        Some(b) => reasons.push(format!("CPU por núcleo: {} puntos (prueba del {}).", b.cpu_single_score, &b.measured_at[..10.min(b.measured_at.len())])),
        None => reasons.push("Todavía no se midió la CPU por núcleo: corré la prueba rápida para comparar esta PC con las demás.".into()),
    }
    // Reserva 3 GB para Windows y las apps; recomienda la mitad de lo restante con tope razonable.
    let usable = ram_total_mb.saturating_sub(3072).max(1024) as u32;
    let recommended = (usable / 2).clamp(2048, 8192) / 512 * 512;
    Assessment {
        rating: match level { 3 => "Buena anfitriona", 2 => "Anfitriona aceptable", _ => "Limitada para hostear" }.into(),
        reasons,
        recommended_server_ram_mb: recommended,
        max_server_ram_mb: usable / 512 * 512,
    }
}

/// Métricas livianas: solo uso de CPU y RAM disponible. Se piden cada pocos segundos
/// únicamente mientras la pantalla correspondiente está abierta.
pub fn live(probe: &Probe, server_ram_need_mb: u64) -> LiveMetrics {
    let mut sys = probe.0.lock().unwrap();
    sys.refresh_cpu_usage();
    sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
    let cpu = sys.global_cpu_usage();
    let avail = sys.available_memory() / 1_048_576;
    let total = sys.total_memory() / 1_048_576;
    let mut reasons = vec![];
    if cpu > 85.0 { reasons.push(format!("La CPU está al {cpu:.0}%: esta PC está ocupada.")); }
    if server_ram_need_mb > 0 && avail < server_ram_need_mb + 1024 {
        reasons.push(format!("Hay {} MB libres; un servidor necesita {} MB más margen.", avail, server_ram_need_mb));
    }
    LiveMetrics { cpu_usage: cpu, ram_available_mb: avail, ram_total_mb: total, host_ready: reasons.is_empty(), reasons }
}

pub fn process_memory_mb(probe: &Probe, pid: u32) -> Option<u64> {
    let mut sys = probe.0.lock().unwrap();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
    sys.process(pid).map(|p| p.memory() / 1_048_576)
}

/// Memoria (MB) y CPU del proceso en % del total de la PC (la CPU es el promedio desde la
/// consulta anterior: la primera vez da 0).
pub fn process_usage(probe: &Probe, pid: u32) -> Option<(u64, f32)> {
    let mut sys = probe.0.lock().unwrap();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory().with_cpu());
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
    sys.process(pid).map(|p| (p.memory() / 1_048_576, (p.cpu_usage() / cores).clamp(0.0, 100.0)))
}

pub fn process_alive(probe: &Probe, pid: u32) -> bool {
    let mut sys = probe.0.lock().unwrap();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
    sys.process(pid).map(|p| p.name().to_string_lossy().to_lowercase().contains("java")).unwrap_or(false)
}

/// Prueba rápida (~2 s): un núcleo de CPU y escritura secuencial en el disco de datos.
/// Se corre al configurar el nodo o cuando el usuario la pide, nunca en segundo plano.
pub fn run_benchmark() -> AppResult<BenchmarkResult> {
    let start = Instant::now();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut acc = 0f64;
    let mut iterations: u64 = 0;
    while start.elapsed().as_millis() < 1500 {
        for _ in 0..10_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            acc += ((x & 0xFFFF) as f64).sqrt() * 1.000_001;
            iterations += 1;
        }
        std::hint::black_box(acc);
    }
    let per_ms = iterations as f64 / start.elapsed().as_millis().max(1) as f64;
    let cpu_single_score = (per_ms / 10.0).round() as u32;

    let disk_write_mb_s = (|| -> Option<u32> {
        use std::io::Write;
        let path = paths::cache().join("bench.tmp");
        let buf = vec![0x5Au8; 4 << 20];
        let t = Instant::now();
        let mut f = std::fs::File::create(&path).ok()?;
        for _ in 0..16 { f.write_all(&buf).ok()?; }
        f.sync_all().ok()?;
        let secs = t.elapsed().as_secs_f64().max(0.001);
        drop(f);
        let _ = std::fs::remove_file(&path);
        Some((64.0 / secs) as u32)
    })();

    let result = BenchmarkResult { cpu_single_score, disk_write_mb_s, measured_at: chrono::Local::now().to_rfc3339() };
    paths::write_json(&bench_path(), &result).map_err(|e| AppError::new("io", "No se pudo guardar el resultado de la prueba.").with_detail(e))?;
    Ok(result)
}

/// Puntaje para elegir anfitriona entre las PCs del grupo (más es mejor). Liviano: usa el
/// benchmark guardado (si existe) y la memoria libre actual; no corre pruebas.
pub fn host_score(probe: &Probe) -> u32 {
    let avail_gb = {
        let mut sys = probe.0.lock().unwrap();
        sys.refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        sys.available_memory() / 1_073_741_824
    };
    let cpu = saved_benchmark().map(|b| b.cpu_single_score).unwrap_or(1000);
    // RAM libre hasta 16 GB (más no suma para un servidor de amigos) y CPU por núcleo.
    (cpu / 10) + (avail_gb.min(16) as u32) * 12
}

/// Pide a Windows que no suspenda la PC (la pantalla se puede apagar igual). El pedido vale para
/// el hilo que lo hace mientras siga vivo: lo usa un hilo dedicado de la app.
pub fn keep_awake(on: bool) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Power::*;
        SetThreadExecutionState(if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
    }
    #[cfg(not(windows))]
    let _ = on;
}
