//! Hardware detection and live GPU sensors. Everything is read locally.
//!
//! * CPU / RAM / OS: `sysinfo`
//! * GPU name + VRAM: Windows display-adapter registry keys (exact 64-bit VRAM size),
//!   falling back to `nvidia-smi` when present.
//! * Live GPU temperature / VRAM use: `nvidia-smi` (NVIDIA). On other vendors the
//!   values are reported as unknown and the temperature guard relies on driver limits.

use crate::util;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, Default)]
pub struct HardwareInfo {
    pub cpu: String,
    pub cpu_cores: usize,
    pub cpu_threads: usize,
    pub ram_gb: f64,
    pub gpu: String,
    pub gpu_vendor: String,
    pub vram_gb: f64,
    pub os: String,
    pub live_sensors: bool,
    /// Short friendly label, e.g. "Strong gaming PC".
    pub tier: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct GpuSample {
    pub temp_c: Option<f64>,
    pub vram_used_gb: Option<f64>,
}

pub fn detect() -> HardwareInfo {
    use sysinfo::System;
    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown processor".into());
    let cpu_threads = sys.cpus().len();
    let cpu_cores = sys.physical_core_count().unwrap_or(cpu_threads);
    let ram_gb = (sys.total_memory() as f64 / 1024f64.powi(3) * 10.0).round() / 10.0;
    let os = format!(
        "{} {}",
        System::name().unwrap_or_else(|| "Windows".into()),
        System::os_version().unwrap_or_default()
    )
    .trim()
    .to_string();

    let (gpu, vram_gb) = detect_gpu();
    let gpu_vendor = vendor_of(&gpu);
    let live_sensors = gpu_vendor == "NVIDIA" && sample_gpu().temp_c.is_some();
    let tier = tier_label(vram_gb, cpu_threads);

    let info = HardwareInfo { cpu, cpu_cores, cpu_threads, ram_gb, gpu, gpu_vendor, vram_gb, os, live_sensors, tier };
    util::log(format!("hardware: {:?}", info));
    info
}

fn vendor_of(name: &str) -> String {
    let n = name.to_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("rtx") || n.contains("gtx") {
        "NVIDIA".into()
    } else if n.contains("amd") || n.contains("radeon") {
        "AMD".into()
    } else if n.contains("intel") || n.contains("arc") {
        "Intel".into()
    } else {
        "Unknown".into()
    }
}

fn tier_label(vram: f64, threads: usize) -> String {
    match (vram, threads) {
        (v, t) if v >= 16.0 && t >= 12 => "High-end gaming PC",
        (v, t) if v >= 10.0 && t >= 8 => "Strong gaming PC",
        (v, _) if v >= 6.0 => "Mid-range gaming PC",
        _ => "Entry-level PC",
    }
    .into()
}

fn detect_gpu() -> (String, f64) {
    #[cfg(windows)]
    if let Some(found) = gpu_from_registry() {
        return found;
    }
    if let Some((name, vram)) = gpu_from_nvidia_smi() {
        return (name, vram);
    }
    ("Unknown graphics card".into(), 0.0)
}

#[cfg(windows)]
fn gpu_from_registry() -> Option<(String, f64)> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    const CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    let root = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(CLASS).ok()?;
    let mut best: Option<(String, f64)> = None;
    for sub in root.enum_keys().flatten() {
        if sub.len() != 4 {
            continue; // "0000", "0001", ... ; skips "Properties"
        }
        let Ok(k) = root.open_subkey(&sub) else { continue };
        let name: String = k.get_value("DriverDesc").unwrap_or_default();
        if name.is_empty() || name.to_lowercase().contains("basic display") {
            continue;
        }
        let bytes: u64 = k
            .get_value::<u64, _>("HardwareInformation.qwMemorySize")
            .ok()
            .or_else(|| k.get_value::<u32, _>("HardwareInformation.MemorySize").ok().map(u64::from))
            .unwrap_or(0);
        let gb = (bytes as f64 / 1024f64.powi(3) * 10.0).round() / 10.0;
        if best.as_ref().map_or(true, |(_, b)| gb > *b) {
            best = Some((name, gb));
        }
    }
    best
}

fn gpu_from_nvidia_smi() -> Option<(String, f64)> {
    let out = util::hidden_command("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?;
    let mut parts = line.split(',').map(|s| s.trim());
    let name = parts.next()?.to_string();
    let mb: f64 = parts.next()?.parse().ok()?;
    Some((name, (mb / 1024.0 * 10.0).round() / 10.0))
}

/// One live reading. Cheap (~30 ms). Returns `None` fields when unavailable.
pub fn sample_gpu() -> GpuSample {
    let Ok(out) = util::hidden_command("nvidia-smi")
        .args(["--query-gpu=temperature.gpu,memory.used", "--format=csv,noheader,nounits"])
        .output()
    else {
        return GpuSample::default();
    };
    if !out.status.success() {
        return GpuSample::default();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.lines().next().unwrap_or("").split(',').map(|s| s.trim().parse::<f64>().ok());
    GpuSample {
        temp_c: parts.next().flatten(),
        vram_used_gb: parts.next().flatten().map(|mb| mb / 1024.0),
    }
}

/// Is any of these processes running? (case-insensitive)
pub fn is_running(names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    sys.processes().values().any(|p| names.iter().any(|n| p.name().eq_ignore_ascii_case(n)))
}

/// Close the game's processes. Returns how many were closed.
pub fn close_processes(names: &[String]) -> usize {
    if names.is_empty() {
        return 0;
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_processes();
    let mut n = 0;
    for p in sys.processes().values() {
        if names.iter().any(|x| p.name().eq_ignore_ascii_case(x)) && p.kill() {
            n += 1;
        }
    }
    util::log(format!("closed {n} process(es) for {:?}", names));
    n
}
